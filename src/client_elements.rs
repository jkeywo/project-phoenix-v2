//! Client-side hideable-element registry.
//!
//! Each console registers named UI elements on spawn. When a complexity
//! preset change is received, the registry hides or shows elements based
//! on the preset's `hidden_elements` list. Unknown names from TOML
//! trigger runtime warnings.

use crate::complexity::parse_complexity_config;
use crate::messages::Console;
use bevy::prelude::Resource;
use std::collections::{HashMap, HashSet};

/// Tracks which element names are registered and which are currently hidden.
#[derive(Clone, Debug, Default, PartialEq, Resource)]
pub struct HideableElementRegistry {
    registered: HashSet<String>,
    hidden: HashSet<String>,
    /// Per-console last-applied preset name so we avoid redundant work.
    pub last_applied: HashMap<Console, String>,
}

impl HideableElementRegistry {
    /// Register an element name.
    pub fn register(&mut self, name: String) {
        self.registered.insert(name);
    }

    /// Whether a name has been registered.
    pub fn is_registered(&self, name: &str) -> bool {
        self.registered.contains(name)
    }

    /// Whether a name is currently marked as hidden.
    pub fn is_hidden(&self, name: &str) -> bool {
        self.hidden.contains(name)
    }

    /// Compute what should change when switching to the given preset.
    ///
    /// Returns names to hide, names to show, and unknown names (in the
    /// preset's TOML `hidden_elements` but not in the registry).
    pub fn planned_changes(&self, console: &Console, preset_name: &str) -> PlannedChanges {
        let to_hide = hideable_element_names(console, preset_name);

        // Everything that was hidden but isn't in the new list should be shown.
        let to_show: Vec<String> = self
            .hidden
            .iter()
            .filter(|n| !to_hide.contains(n))
            .cloned()
            .collect();

        // Names in the preset that aren't registered.
        let unknown: Vec<String> = to_hide
            .iter()
            .filter(|n| !self.registered.contains(*n))
            .cloned()
            .collect();

        PlannedChanges {
            to_hide,
            to_show,
            unknown,
        }
    }

    /// Apply a set of planned changes (updates the `hidden` set).
    pub fn apply_changes(&mut self, changes: &PlannedChanges) {
        for name in &changes.to_hide {
            self.hidden.insert(name.clone());
        }
        for name in &changes.to_show {
            self.hidden.remove(name);
        }
    }
}

/// The result of [`HideableElementRegistry::planned_changes`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedChanges {
    /// Element names to hide (set `Display::None`).
    pub to_hide: Vec<String>,
    /// Element names to restore (set `Display::Flex`).
    pub to_show: Vec<String>,
    /// Names from TOML `hidden_elements` that are not in the registry.
    pub unknown: Vec<String>,
}

/// Returns the element names that the given preset wants to hide.
///
/// Loaded from the complexity TOML file embedded at compile time.
/// Returns an empty vec for any console that has no complexity config
/// or for preset names that don't exist in the config.
pub fn hideable_element_names(console: &Console, preset_name: &str) -> Vec<String> {
    let toml_str = match console {
        Console::Tactical => include_str!("../assets/complexity/tactical.toml"),
        Console::Science => include_str!("../assets/complexity/science.toml"),
        Console::Power => include_str!("../assets/complexity/power.toml"),
        _ => return vec![],
    };
    let Ok(config) = parse_complexity_config(toml_str) else {
        return vec![];
    };
    config
        .get_preset(preset_name)
        .map(|p| p.hidden_elements.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Registration ──────────────────────────────────────────────

    #[test]
    fn register_adds_name() {
        let mut r = HideableElementRegistry::default();
        r.register("phaser_mode_selector".into());
        assert!(r.is_registered("phaser_mode_selector"));
        assert!(!r.is_registered("torpedo_tube_selector"));
    }

    #[test]
    fn register_is_idempotent() {
        let mut r = HideableElementRegistry::default();
        r.register("foo".into());
        r.register("foo".into());
        assert!(r.is_registered("foo"));
    }

    // ── Planned changes: Tactical Low ──────────────────────────────

    #[test]
    fn tactical_low_preset_returns_hidden_elements() {
        let names = hideable_element_names(&Console::Tactical, "Low");
        assert!(names.contains(&"phaser_mode_selector".to_string()));
        assert!(names.contains(&"torpedo_tube_selector".to_string()));
        assert!(names.contains(&"target_lock_button".to_string()));
    }

    #[test]
    fn tactical_low_all_registered_hides_three_shows_none() {
        let mut r = HideableElementRegistry::default();
        r.register("phaser_mode_selector".into());
        r.register("torpedo_tube_selector".into());
        r.register("target_lock_button".into());

        let changes = r.planned_changes(&Console::Tactical, "Low");
        assert_eq!(changes.to_hide.len(), 3);
        assert!(changes.to_hide.contains(&"phaser_mode_selector".to_string()));
        assert!(changes.to_hide.contains(&"torpedo_tube_selector".to_string()));
        assert!(changes.to_hide.contains(&"target_lock_button".to_string()));
        assert!(changes.to_show.is_empty());
        assert!(changes.unknown.is_empty());
    }

    #[test]
    fn tactical_low_reports_unregistered_elements() {
        let r = HideableElementRegistry::default(); // nothing registered
        let changes = r.planned_changes(&Console::Tactical, "Low");
        assert_eq!(changes.unknown.len(), 3);
        assert!(changes.unknown.contains(&"phaser_mode_selector".to_string()));
        assert!(changes.unknown.contains(&"torpedo_tube_selector".to_string()));
        assert!(changes.unknown.contains(&"target_lock_button".to_string()));
    }

    // ── Planned changes: Tactical Full ─────────────────────────────

    #[test]
    fn tactical_full_hides_nothing() {
        let r = HideableElementRegistry::default();
        let changes = r.planned_changes(&Console::Tactical, "Std");
        assert!(changes.to_hide.is_empty());
        assert!(changes.to_show.is_empty());
        assert!(changes.unknown.is_empty());
    }

    #[test]
    fn tactical_full_shows_previously_hidden_elements() {
        let mut r = HideableElementRegistry::default();
        r.register("phaser_mode_selector".into());
        r.register("torpedo_tube_selector".into());
        r.register("target_lock_button".into());

        // Start hidden as if Low was applied.
        let low = r.planned_changes(&Console::Tactical, "Low");
        r.apply_changes(&low);

        assert!(r.is_hidden("phaser_mode_selector"));
        assert!(r.is_hidden("torpedo_tube_selector"));

        // Now switch to Full.
        let full = r.planned_changes(&Console::Tactical, "Std");
        assert!(full.to_hide.is_empty());
        assert_eq!(full.to_show.len(), 3);
        assert!(full.to_show.contains(&"phaser_mode_selector".to_string()));
        assert!(full.to_show.contains(&"torpedo_tube_selector".to_string()));
        assert!(full.to_show.contains(&"target_lock_button".to_string()));
    }

    // ── Apply changes ─────────────────────────────────────────────

    #[test]
    fn apply_changes_updates_hidden_set() {
        let mut r = HideableElementRegistry::default();
        r.register("a".into());
        r.register("b".into());

        r.apply_changes(&PlannedChanges {
            to_hide: vec!["a".into()],
            to_show: vec![],
            unknown: vec![],
        });
        assert!(r.is_hidden("a"));
        assert!(!r.is_hidden("b"));

        // Show a again
        r.apply_changes(&PlannedChanges {
            to_hide: vec![],
            to_show: vec!["a".into()],
            unknown: vec![],
        });
        assert!(!r.is_hidden("a"));
    }

    // ── Science Low ───────────────────────────────────────────────

    #[test]
    fn science_low_hides_shield_frequency_readout() {
        let names = hideable_element_names(&Console::Science, "Low");
        assert!(
            names.contains(&"shield_frequency_readout".to_string()),
            "Science Low must hide shield_frequency_readout"
        );
    }

    #[test]
    fn science_full_hides_nothing() {
        let names = hideable_element_names(&Console::Science, "Std");
        assert!(names.is_empty(), "Science Full should hide nothing");
    }

    // ── Non-Tactical/Science consoles ─────────────────────────────

    #[test]
    fn non_tactical_science_power_consoles_without_config_return_empty() {
        for console in &[Console::Helm, Console::CaptainChair, Console::Repair] {
            let names = hideable_element_names(console, "Low");
            assert!(names.is_empty(), "{:?} should have no hidden elements", console);
        }
    }

    // ── Power Low ─────────────────────────────────────────────────

    #[test]
    fn power_low_hides_overflow_controls() {
        let names = hideable_element_names(&Console::Power, "Low");
        assert!(
            names.contains(&"power_overflow_controls".to_string()),
            "Power Low must hide power_overflow_controls"
        );
    }

    #[test]
    fn power_full_hides_nothing() {
        let names = hideable_element_names(&Console::Power, "Std");
        assert!(names.is_empty(), "Power Full should hide nothing");
    }

    #[test]
    fn helm_is_not_affected_by_tactical_low() {
        let r = HideableElementRegistry::default();
        let changes = r.planned_changes(&Console::Helm, "Low");
        assert!(changes.to_hide.is_empty());
        assert!(changes.to_show.is_empty());
    }

    // ── Unknown preset name ───────────────────────────────────────

    #[test]
    fn unknown_preset_name_returns_empty() {
        let r = HideableElementRegistry::default();
        let changes = r.planned_changes(&Console::Tactical, "NonExistent");
        assert!(changes.to_hide.is_empty());
    }

    // ── End-to-end: Low→Full cycle ────────────────────────────────

    #[test]
    fn low_to_full_cycle_hides_then_shows() {
        let mut r = HideableElementRegistry::default();
        r.register("phaser_mode_selector".into());
        r.register("torpedo_tube_selector".into());

        // Apply Low
        let low = r.planned_changes(&Console::Tactical, "Low");
        assert!(low.unknown.contains(&"target_lock_button".to_string()));
        r.apply_changes(&low);

        assert!(r.is_hidden("phaser_mode_selector"));
        assert!(r.is_hidden("torpedo_tube_selector"));

        // Apply Full
        let full = r.planned_changes(&Console::Tactical, "Std");
        assert!(full.unknown.is_empty());
        r.apply_changes(&full);

        assert!(!r.is_hidden("phaser_mode_selector"));
        assert!(!r.is_hidden("torpedo_tube_selector"));
    }
}
