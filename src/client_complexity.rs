//! Pure client-side complexity preset selection state.
//!
//! Manages the per-console complexity choice lifecycle: first-use pop-up,
//! dropdown visibility, stale-preset re-prompt, and effective preset
//! resolution. Bevy-free; fully unit-testable on native.

use crate::messages::{ClientMessage, Console};
use std::collections::HashMap;

/// The lifecycle state of complexity preset choice for a single console.
#[derive(Clone, Debug, PartialEq)]
pub struct ComplexityChoice {
    /// Available preset names, in display order.
    pub available_presets: Vec<String>,
    /// The chosen preset name, if the player has made a selection.
    pub chosen: Option<String>,
    /// Whether the first-use pop-up has been shown (or dismissed) this
    /// session. Reset when a fresh pop-up is needed (e.g. on stale-preset
    /// re-prompt).
    pub popup_shown: bool,
}

impl ComplexityChoice {
    /// Create a new choice state from available presets and an optional
    /// stored preset from a previous session.
    pub fn new(available_presets: Vec<String>, stored: Option<String>) -> Self {
        let popup_shown = stored.is_some();
        Self {
            available_presets,
            chosen: stored,
            popup_shown,
        }
    }

    /// The preset name that should be active for this console.
    ///
    /// Returns the player's chosen preset, or `"Low"` if unset (the default
    /// selection on the first-use pop-up), or `"Full"` if there's only one
    /// preset.
    pub fn effective_preset(&self) -> &str {
        if let Some(ref c) = self.chosen {
            return c;
        }
        if self.available_presets.len() == 1 {
            return &self.available_presets[0];
        }
        "Low"
    }

    /// True when the console has more than one preset to choose from.
    pub fn show_dropdown(&self) -> bool {
        self.available_presets.len() > 1
    }

    /// True when a first-use or re-prompt pop-up should be displayed.
    ///
    /// A pop-up is shown when the player has not yet made a choice AND
    /// there are multiple presets available (so there is a meaningful
    /// choice to make).
    pub fn show_popup(&self) -> bool {
        self.show_dropdown() && !self.popup_shown && self.chosen.is_none()
    }

    /// True when the stored preset name is not in the available list.
    ///
    /// When stale, the caller should re-trigger the first-use pop-up by
    /// creating a new `ComplexityChoice` via `new()` (which clears the
    /// stored choice and resets the pop-up flag).
    pub fn is_stale(&self) -> bool {
        if let Some(ref c) = self.chosen {
            !self.available_presets.iter().any(|p| p == c)
        } else {
            false
        }
    }

    /// Select a preset by name. Returns `Err(name)` if the name is not
    /// in the available list.
    pub fn select(&mut self, name: &str) -> Result<(), String> {
        if !self.available_presets.iter().any(|p| p == name) {
            return Err(name.to_string());
        }
        self.chosen = Some(name.to_string());
        self.popup_shown = true;
        Ok(())
    }
}

/// Default available presets for the Tactical console (Low + Full).
pub fn tactical_available_presets() -> Vec<String> {
    vec!["Low".into(), "Full".into()]
}

/// Build a `SetComplexity` message for the given console and preset name.
pub fn set_complexity_message(console: Console, preset_name: &str) -> ClientMessage {
    ClientMessage::SetComplexity {
        console,
        preset_name: preset_name.to_string(),
    }
}

/// Per-console complexity preset selections, keyed by Console.
///
/// Inserted as a Bevy resource so the UI layer can read it when
/// rendering console panels.
#[derive(Clone, Debug, PartialEq, bevy::prelude::Resource)]
pub struct ComplexityStore {
    pub choices: HashMap<Console, ComplexityChoice>,
}

impl ComplexityStore {
    pub fn new() -> Self {
        Self {
            choices: HashMap::new(),
        }
    }

    /// Get the choice state for a console, creating a default one (single
    /// "Full" preset, or ["Low","Full"] for Tactical/Science) if none exists.
    pub fn for_console(&mut self, console: &Console) -> &mut ComplexityChoice {
        let presets = if *console == Console::Tactical || *console == Console::Science {
            vec!["Low".to_string(), "Full".to_string()]
        } else {
            vec!["Full".to_string()]
        };
        self.choices.entry(console.clone()).or_insert_with(|| {
            ComplexityChoice::new(presets, None)
        })
    }
}

impl ComplexityStore {
    /// Apply stored complexity presets (from localStorage via JS bridge).
    ///
    /// Only applies presets where the stored name is valid (in the current
    /// available list). Stale names are discarded so the UI shows the
    /// first-use pop-up again.
    pub fn apply_stored(&mut self, stored: &HashMap<Console, String>) {
        for (console, preset_name) in stored {
            let choice = self.for_console(console);
            if choice.available_presets.iter().any(|p| p == preset_name) {
                let _ = choice.select(preset_name);
            }
            // Stale: leave unchosen so pop-up re-triggers.
        }
    }
}

impl Default for ComplexityStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle 1: Multiple presets, no choice ─────────────────────────

    #[test]
    fn multiple_presets_no_choice_shows_popup_and_dropdown_defaults_to_low() {
        let c = ComplexityChoice::new(vec!["Low".into(), "Full".into()], None);
        assert!(c.show_dropdown(), "dropdown shown when >1 preset");
        assert!(c.show_popup(), "pop-up shown when no choice yet");
        assert_eq!(c.effective_preset(), "Low", "default effective preset is Low");
    }

    // ── Cycle 2: Stored choice → no popup, effective = chosen ─────────

    #[test]
    fn stored_choice_uses_chosen_preset_and_suppresses_popup() {
        let c = ComplexityChoice::new(vec!["Low".into(), "Full".into()], Some("Full".into()));
        assert!(c.show_dropdown(), "dropdown still shown when >1 preset");
        assert!(!c.show_popup(), "no popup when choice already stored");
        assert_eq!(c.effective_preset(), "Full", "effective preset is the stored one");
    }

    #[test]
    fn select_updates_chosen_and_marks_popup_shown() {
        let mut c = ComplexityChoice::new(vec!["Low".into(), "Full".into()], None);
        assert!(c.select("Full").is_ok());
        assert_eq!(c.chosen, Some("Full".into()));
        assert!(c.popup_shown);
        assert!(!c.show_popup());
    }

    #[test]
    fn select_invalid_name_returns_error() {
        let mut c = ComplexityChoice::new(vec!["Low".into(), "Full".into()], None);
        assert!(c.select("High").is_err());
        assert!(c.chosen.is_none());
    }

    // ── Cycle 3: Single preset → no dropdown, no popup ──────────────

    #[test]
    fn single_preset_hides_dropdown_and_popup() {
        let c = ComplexityChoice::new(vec!["Full".into()], None);
        assert!(!c.show_dropdown(), "no dropdown with only one preset");
        assert!(!c.show_popup(), "no popup with only one preset");
        assert_eq!(c.effective_preset(), "Full", "effective preset is the only one");
    }

    #[test]
    fn single_preset_with_stored_still_hides_dropdown() {
        let c = ComplexityChoice::new(vec!["Full".into()], Some("Full".into()));
        assert!(!c.show_dropdown());
        assert!(!c.show_popup());
        assert_eq!(c.effective_preset(), "Full");
    }

    // ── Message builder ─────────────────────────────────────────────

    #[test]
    fn set_complexity_message_builder_produces_correct_message() {
        let msg = set_complexity_message(Console::Tactical, "Low");
        assert_eq!(msg, ClientMessage::SetComplexity {
            console: Console::Tactical,
            preset_name: "Low".into(),
        });
    }

    // ── Cycle 4: Stale preset detection + re-prompt ─────────────────

    #[test]
    fn stale_preset_detected_when_chosen_not_in_available() {
        let c = ComplexityChoice::new(vec!["Low".into(), "Full".into()], Some("High".into()));
        assert!(c.is_stale(), "stored 'High' is not in available list");
        // A stale choice should re-trigger pop-up when reconstructed.
        let fresh = ComplexityChoice::new(vec!["Low".into(), "Full".into()], None);
        assert!(fresh.show_popup(), "fresh start triggers popup");
    }

    #[test]
    fn not_stale_when_chosen_in_available() {
        let c = ComplexityChoice::new(vec!["Low".into(), "Full".into()], Some("Full".into()));
        assert!(!c.is_stale());
    }

    #[test]
    fn not_stale_when_no_choice() {
        let c = ComplexityChoice::new(vec!["Low".into(), "Full".into()], None);
        assert!(!c.is_stale());
    }

    // ── ComplexityStore ─────────────────────────────────────────────

    #[test]
    fn store_for_console_creates_tactical_with_low_full() {
        let mut store = ComplexityStore::new();
        let choice = store.for_console(&Console::Tactical);
        assert_eq!(choice.available_presets, vec!["Low", "Full"]);
        assert!(choice.show_dropdown());
    }

    #[test]
    fn store_for_console_creates_science_with_low_full() {
        let mut store = ComplexityStore::new();
        let choice = store.for_console(&Console::Science);
        assert_eq!(choice.available_presets, vec!["Low", "Full"]);
        assert!(choice.show_dropdown(), "Science should show complexity dropdown");
    }

    #[test]
    fn store_for_console_creates_other_with_single_full() {
        let mut store = ComplexityStore::new();
        let choice = store.for_console(&Console::Helm);
        assert_eq!(choice.available_presets, vec!["Full"]);
        assert!(!choice.show_dropdown());
    }

    #[test]
    fn store_apply_stored_valid_preset_updates_choice() {
        let mut store = ComplexityStore::new();
        let mut stored = HashMap::new();
        stored.insert(Console::Tactical, "Full".to_string());
        store.apply_stored(&stored);
        let choice = store.for_console(&Console::Tactical);
        assert_eq!(choice.effective_preset(), "Full");
        assert!(!choice.show_popup(), "valid stored preset suppresses popup");
    }

    #[test]
    fn store_apply_stored_stale_preset_discarded() {
        let mut store = ComplexityStore::new();
        let mut stored = HashMap::new();
        stored.insert(Console::Tactical, "High".to_string());
        store.apply_stored(&stored);
        let choice = store.for_console(&Console::Tactical);
        assert!(choice.chosen.is_none(), "stale preset should be discarded");
        assert!(choice.show_popup(), "stale preserved triggers re-prompt");
    }

    #[test]
    fn resolving_stale_preset_creates_fresh_choice() {
        // Simulate: player had "High" stored, but TOML only has Low/Full.
        let stale = ComplexityChoice::new(vec!["Low".into(), "Full".into()], Some("High".into()));
        assert!(stale.is_stale());
        // On re-prompt: create a new ComplexityChoice without the stale stored value.
        let fresh = ComplexityChoice::new(vec!["Low".into(), "Full".into()], None);
        assert!(fresh.show_popup(), "re-prompt triggers popup");
        assert_eq!(fresh.effective_preset(), "Low", "default back to Low");
    }
}
