//! First-valid-wins scenario + player-ship selection, as a pure rule set
//! (issue #755's arbiter, ported to Rust for issue #1326).
//!
//! # Why this exists in Rust at all
//!
//! On the browser host the arbiter is JavaScript — `gui/scenario-arbiter.js`,
//! driven by `server.html`, which intercepts
//! [`ClientMessage::SelectScenario`](crate::core::messages::ClientMessage::SelectScenario)
//! and
//! [`SelectPlayerShip`](crate::core::messages::ClientMessage::SelectPlayerShip)
//! on both the datachannel and the local-console path and never forwards them
//! to WASM. It can be JavaScript there because the browser host has no Bevy
//! `App` yet when the choice is made: `wasm_init` throws to unwind the JS stack
//! and therefore has to run *after* `wasm_load_world`.
//!
//! A native host has no JavaScript and, since issue #1326, no such ordering
//! escape either: its window and its lobby exist before a world is chosen, so
//! the arbitration has to happen inside the running process. This module is
//! that arbitration, and it is deliberately a **transcription** of
//! `gui/scenario-arbiter.js` rather than a second design:
//!
//! | `gui/scenario-arbiter.js` | here |
//! |---|---|
//! | `normalizeSelection` | [`ScenarioSelection::default`] |
//! | `findScenario` | [`find_scenario`] |
//! | `selectScenario` | [`select_scenario`] |
//! | `selectPlayerShip` | [`select_player_ship`] |
//! | `isComplete` | [`ScenarioSelection::is_complete`] |
//! | `worldPathFor` | [`world_path_for`] |
//! | `curatedShipsFor` | [`curated_ships_for`] |
//!
//! The rules, restated so a reader need not open the JS: **the first request
//! that validates against the pre-load catalogue wins.** There is no voting and
//! no pre-ship captain authority — a phone and the host's own surface are equal
//! senders. A scenario id the catalogue does not list is *rejected*; a request
//! that arrives once the value is already locked is *ignored*. A ship may only
//! be chosen once a scenario is locked (hulls are scoped to their scenario,
//! issue #754 AC4) and must be one that scenario's catalogue entry offers —
//! which is already the `--manifest` curation filter, applied by
//! [`build_catalog`](crate::world::manifest::build_catalog).
//!
//! Pure and Bevy-free (AGENTS.md rule 10): the Bevy adapter that feeds it
//! inbound messages and acts on a completed selection is
//! [`crate::native_host::world_load`].

use crate::core::messages::ScenarioCatalogWire;
use crate::world::manifest::{ScenarioCatalog, ScenarioCatalogEntry};

/// What the host has locked so far. Both fields start `None`; each is written
/// at most once, by the first request that validates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenarioSelection {
    /// The locked scenario's catalogue id (`[[scenario]] id` in the manifest).
    pub scenario_id: Option<String>,
    /// The locked player hull's `template_path`.
    pub template_path: Option<String>,
}

impl ScenarioSelection {
    /// True once both a scenario and a hull are locked — the moment the world
    /// may be loaded. `isComplete` in the JS.
    pub fn is_complete(&self) -> bool {
        self.scenario_id.is_some() && self.template_path.is_some()
    }
}

/// What one selection request did.
///
/// Named for the JS's three outcome strings so the two implementations can be
/// read against each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionOutcome {
    /// The request locked a new value; the returned selection replaces the held
    /// one.
    Accepted,
    /// Already locked. Deliberately not an error: a phone that pressed the same
    /// button a moment too late is not misbehaving.
    Ignored,
    /// Failed catalogue validation — an unknown scenario id, a hull this
    /// scenario does not offer, or a hull request before any scenario is locked.
    Rejected,
}

/// Find a catalogue entry by scenario id. `findScenario` in the JS.
pub fn find_scenario<'a>(
    catalog: &'a ScenarioCatalog,
    scenario_id: &str,
) -> Option<&'a ScenarioCatalogEntry> {
    catalog.scenarios.iter().find(|s| s.id == scenario_id)
}

/// Apply a scenario request to `selection`, returning the outcome and the
/// selection that should be held afterwards.
///
/// The selection is returned rather than mutated in place so a rejected request
/// cannot half-write the held state — the JS returns a fresh object for the
/// same reason.
pub fn select_scenario(
    selection: &ScenarioSelection,
    catalog: &ScenarioCatalog,
    scenario_id: &str,
) -> (SelectionOutcome, ScenarioSelection) {
    if selection.scenario_id.is_some() {
        return (SelectionOutcome::Ignored, selection.clone());
    }
    if find_scenario(catalog, scenario_id).is_none() {
        return (SelectionOutcome::Rejected, selection.clone());
    }
    (
        SelectionOutcome::Accepted,
        ScenarioSelection {
            scenario_id: Some(scenario_id.to_string()),
            // Deliberately clears the hull, exactly as the JS does: a hull is
            // only meaningful against the scenario that offered it.
            template_path: None,
        },
    )
}

/// Apply a player-hull request. Requires a locked scenario, and the hull must
/// be one that scenario's catalogue entry offers.
pub fn select_player_ship(
    selection: &ScenarioSelection,
    catalog: &ScenarioCatalog,
    template_path: &str,
) -> (SelectionOutcome, ScenarioSelection) {
    let Some(scenario_id) = selection.scenario_id.as_deref() else {
        return (SelectionOutcome::Rejected, selection.clone());
    };
    if selection.template_path.is_some() {
        return (SelectionOutcome::Ignored, selection.clone());
    }
    let Some(entry) = find_scenario(catalog, scenario_id) else {
        return (SelectionOutcome::Rejected, selection.clone());
    };
    if !entry.ships.iter().any(|s| s.template_path == template_path) {
        return (SelectionOutcome::Rejected, selection.clone());
    }
    (
        SelectionOutcome::Accepted,
        ScenarioSelection {
            scenario_id: Some(scenario_id.to_string()),
            template_path: Some(template_path.to_string()),
        },
    )
}

/// The world TOML path the locked scenario names, or `None` while nothing is
/// locked. `worldPathFor` in the JS.
pub fn world_path_for<'a>(
    catalog: &'a ScenarioCatalog,
    selection: &ScenarioSelection,
) -> Option<&'a str> {
    let id = selection.scenario_id.as_deref()?;
    find_scenario(catalog, id).map(|entry| entry.world.as_str())
}

/// The hulls the locked scenario's catalogue entry offers — issue #917's
/// curated allowlist, in the world's own authored order.
///
/// This is exactly what `wasm_load_world`'s `curated_ships` argument carries in
/// the browser, and what [`NativeHostConfig::curated_ships`] carries natively:
/// empty means unrestricted, matching
/// [`ScenarioEntry::ships`](crate::world::manifest::ScenarioEntry)'s own
/// semantics.
///
/// [`NativeHostConfig::curated_ships`]: crate::native_host::NativeHostConfig::curated_ships
pub fn curated_ships_for(catalog: &ScenarioCatalog, selection: &ScenarioSelection) -> Vec<String> {
    let Some(id) = selection.scenario_id.as_deref() else {
        return Vec::new();
    };
    find_scenario(catalog, id)
        .map(|entry| {
            entry
                .ships
                .iter()
                .map(|s| s.template_path.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Render a catalogue for the crew wire.
///
/// The Rust twin of `server.html`'s `scenarioCatalogMessage()`, which builds the
/// same [`ServerMessage::ScenarioCatalog`](crate::core::messages::ServerMessage::ScenarioCatalog)
/// payload out of the JS array `wasm_get_scenario_catalog` returns, so a phone's
/// `gui/lobby-state.js` folds a native host's catalogue exactly as it folds a
/// browser host's.
pub fn catalog_wire(catalog: &ScenarioCatalog) -> Vec<ScenarioCatalogWire> {
    catalog
        .scenarios
        .iter()
        .map(|entry| ScenarioCatalogWire {
            id: entry.id.clone(),
            world: entry.world.clone(),
            label: entry.label.clone(),
            description: entry.description.clone(),
            ships: entry.ships.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::config::AvailableShipEntry;

    fn ship(path: &str) -> AvailableShipEntry {
        AvailableShipEntry {
            template_path: path.to_string(),
            label: None,
        }
    }

    fn entry(id: &str, world: &str, ships: &[&str]) -> ScenarioCatalogEntry {
        ScenarioCatalogEntry {
            id: id.to_string(),
            world: world.to_string(),
            label: Some(format!("{id} label")),
            description: None,
            ships: ships.iter().map(|p| ship(p)).collect(),
            origin: None,
        }
    }

    fn catalog() -> ScenarioCatalog {
        ScenarioCatalog {
            scenarios: vec![
                entry(
                    "combat_test",
                    "assets/worlds/combat_test.toml",
                    &[
                        "assets/entities/alliance_destroyer.toml",
                        "assets/entities/alliance_cruiser.toml",
                    ],
                ),
                entry(
                    "patrol",
                    "assets/worlds/patrol.toml",
                    &["assets/entities/alliance_cruiser.toml"],
                ),
            ],
        }
    }

    #[test]
    fn the_first_valid_scenario_request_locks_it() {
        let (outcome, selection) =
            select_scenario(&ScenarioSelection::default(), &catalog(), "patrol");
        assert_eq!(outcome, SelectionOutcome::Accepted);
        assert_eq!(selection.scenario_id.as_deref(), Some("patrol"));
        assert!(selection.template_path.is_none());
    }

    #[test]
    fn a_later_scenario_request_is_ignored_rather_than_refused() {
        let locked = ScenarioSelection {
            scenario_id: Some("patrol".into()),
            template_path: None,
        };
        let (outcome, selection) = select_scenario(&locked, &catalog(), "combat_test");
        assert_eq!(outcome, SelectionOutcome::Ignored);
        assert_eq!(
            selection, locked,
            "an ignored request must not move the held selection"
        );
    }

    #[test]
    fn an_unlisted_scenario_is_rejected() {
        let (outcome, selection) =
            select_scenario(&ScenarioSelection::default(), &catalog(), "not_a_scenario");
        assert_eq!(outcome, SelectionOutcome::Rejected);
        assert_eq!(selection, ScenarioSelection::default());
    }

    #[test]
    fn a_hull_before_a_scenario_is_rejected() {
        let (outcome, _) = select_player_ship(
            &ScenarioSelection::default(),
            &catalog(),
            "assets/entities/alliance_cruiser.toml",
        );
        assert_eq!(
            outcome,
            SelectionOutcome::Rejected,
            "hulls are scoped to their scenario (issue #754 AC4)"
        );
    }

    #[test]
    fn a_hull_the_locked_scenario_does_not_offer_is_rejected() {
        let locked = ScenarioSelection {
            scenario_id: Some("patrol".into()),
            template_path: None,
        };
        let (outcome, _) = select_player_ship(
            &locked,
            &catalog(),
            "assets/entities/alliance_destroyer.toml",
        );
        assert_eq!(
            outcome,
            SelectionOutcome::Rejected,
            "patrol's catalogue entry offers only the cruiser"
        );
    }

    #[test]
    fn a_hull_the_locked_scenario_offers_completes_the_selection() {
        let locked = ScenarioSelection {
            scenario_id: Some("combat_test".into()),
            template_path: None,
        };
        let (outcome, selection) = select_player_ship(
            &locked,
            &catalog(),
            "assets/entities/alliance_destroyer.toml",
        );
        assert_eq!(outcome, SelectionOutcome::Accepted);
        assert!(selection.is_complete());
        assert_eq!(
            world_path_for(&catalog(), &selection),
            Some("assets/worlds/combat_test.toml")
        );
    }

    #[test]
    fn a_second_hull_request_is_ignored() {
        let locked = ScenarioSelection {
            scenario_id: Some("combat_test".into()),
            template_path: Some("assets/entities/alliance_destroyer.toml".into()),
        };
        let (outcome, selection) =
            select_player_ship(&locked, &catalog(), "assets/entities/alliance_cruiser.toml");
        assert_eq!(outcome, SelectionOutcome::Ignored);
        assert_eq!(selection, locked);
    }

    #[test]
    fn a_scenario_lock_leaves_the_selection_incomplete() {
        let (_, selection) = select_scenario(&ScenarioSelection::default(), &catalog(), "patrol");
        assert!(!selection.is_complete());
        assert!(
            world_path_for(&catalog(), &selection).is_some(),
            "the world is resolvable from the scenario alone"
        );
    }

    #[test]
    fn curated_ships_are_the_locked_scenarios_offered_hulls_in_order() {
        let locked = ScenarioSelection {
            scenario_id: Some("combat_test".into()),
            template_path: None,
        };
        assert_eq!(
            curated_ships_for(&catalog(), &locked),
            vec![
                "assets/entities/alliance_destroyer.toml".to_string(),
                "assets/entities/alliance_cruiser.toml".to_string(),
            ]
        );
        assert!(
            curated_ships_for(&catalog(), &ScenarioSelection::default()).is_empty(),
            "nothing locked means unrestricted, matching ScenarioEntry::ships"
        );
    }

    #[test]
    fn the_wire_catalogue_carries_every_entry_and_its_hulls() {
        let wire = catalog_wire(&catalog());
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0].id, "combat_test");
        assert_eq!(wire[0].world, "assets/worlds/combat_test.toml");
        assert_eq!(wire[0].label.as_deref(), Some("combat_test label"));
        assert_eq!(
            wire[0]
                .ships
                .iter()
                .map(|s| s.template_path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "assets/entities/alliance_destroyer.toml",
                "assets/entities/alliance_cruiser.toml"
            ]
        );
    }
}
