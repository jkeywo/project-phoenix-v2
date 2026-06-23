use crate::stations_config::StationAssignments;

// reassign_on_join, reassign_on_leave, and all per-count cascade helpers
// removed in B3 (issue #533). Station layout is now a fixed flat list
// populated from ShipConfigResource.

/// Maps session token → station name.  A token absent from this map is a spectator.
pub type StationAssignmentsAlias = StationAssignments;

#[cfg(test)]
mod tests {
    use crate::messages::Console;
    use crate::stations_config::default_complexity_presets;

    // ── Std preset name (issue #303: TOML files declare "Std", not "Full") ──

    #[test]
    fn default_complexity_presets_uses_std_not_full() {
        let presets = default_complexity_presets();
        for (console, names) in &presets {
            assert!(
                !names.iter().any(|n| n == "Full"),
                "console {:?} still advertises 'Full' preset — should be 'Std'",
                console
            );
            assert!(
                names.iter().any(|n| n == "Std"),
                "console {:?} is missing 'Std' preset",
                console
            );
        }
    }

    #[test]
    fn default_complexity_presets_sensors_only_std() {
        let presets = default_complexity_presets();
        let got = presets
            .get(&Console::Sensors)
            .expect("Sensors should have presets");
        assert_eq!(got.len(), 1, "Sensors should have exactly one preset");
        assert_eq!(got[0], "Std", "Sensors preset should be 'Std'");
    }

    #[test]
    fn default_complexity_presets_shields_only_std() {
        let presets = default_complexity_presets();
        let got = presets
            .get(&Console::Shields)
            .expect("Shields should have presets");
        assert_eq!(got.len(), 1, "Shields should have exactly one preset");
        assert_eq!(got[0], "Std", "Shields preset should be 'Std'");
    }

    #[test]
    fn default_complexity_presets_navigation_only_std() {
        let presets = default_complexity_presets();
        let got = presets
            .get(&Console::Navigation)
            .expect("Navigation should have presets");
        assert_eq!(got.len(), 1, "Navigation preset should be 'Std'");
        assert_eq!(got[0], "Std", "Navigation preset should be 'Std'");
    }
}
