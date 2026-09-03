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
//! | `normalizeSelection` | [`ScenarioSelection::normalized`] |
//! | `findScenario` | [`find_scenario`] |
//! | `selectScenario` | [`select_scenario`] |
//! | `selectPlayerShip` | [`select_player_ship`] |
//! | `isComplete` | [`ScenarioSelection::is_complete`] |
//! | `worldPathFor` | [`world_path_for`] |
//! | `curatedShipsFor` | [`curated_ships_for`] |
//!
//! # The table that keeps them honest
//!
//! A transcription held together by a doc comment drifts, and this one had
//! already: `normalizeSelection` was mapped to `Default` — i.e. to nothing —
//! and its rule that a **falsy field is not a lock** was missing here, so an
//! empty id locked a native host and left a browser host open.
//!
//! `tests/fixtures/scenario-arbiter-parity.json` is the fix: one case table
//! read by this module's own tests AND by
//! `tests/client/scenario-arbiter-parity.test.js`, which drives the JS. Neither
//! side owns the cases, and a case only one side can satisfy is a bug in that
//! side rather than a reason to fork the table.
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
    /// The locked scenario id, or `None` while nothing is locked.
    ///
    /// An **empty string is not a lock**. That is `normalizeSelection`'s rule in
    /// the JS — every field is coerced through `|| null`, so `''` reads as
    /// unlocked — and it was the one line of the arbiter this transcription
    /// originally left out. Without it a manifest entry with an empty `id` locks
    /// a native host into a selection the browser host would still treat as
    /// open, and the two arbiters answer the same request differently.
    pub fn scenario(&self) -> Option<&str> {
        self.scenario_id.as_deref().filter(|id| !id.is_empty())
    }

    /// The locked hull's `template_path`, under [`scenario`](Self::scenario)'s
    /// rule.
    pub fn ship(&self) -> Option<&str> {
        self.template_path.as_deref().filter(|p| !p.is_empty())
    }

    /// This selection with every falsy field coerced to `None` —
    /// `normalizeSelection` in the JS, which returns the normalised object on
    /// *every* path including `ignored` and `rejected`.
    pub fn normalized(&self) -> ScenarioSelection {
        ScenarioSelection {
            scenario_id: self.scenario().map(str::to_string),
            template_path: self.ship().map(str::to_string),
        }
    }

    /// True once both a scenario and a hull are locked — the moment the world
    /// may be loaded. `isComplete` in the JS.
    pub fn is_complete(&self) -> bool {
        self.scenario().is_some() && self.ship().is_some()
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
    if selection.scenario().is_some() {
        return (SelectionOutcome::Ignored, selection.normalized());
    }
    if find_scenario(catalog, scenario_id).is_none() {
        return (SelectionOutcome::Rejected, selection.normalized());
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
    let Some(scenario_id) = selection.scenario() else {
        return (SelectionOutcome::Rejected, selection.normalized());
    };
    if selection.ship().is_some() {
        return (SelectionOutcome::Ignored, selection.normalized());
    }
    let Some(entry) = find_scenario(catalog, scenario_id) else {
        return (SelectionOutcome::Rejected, selection.normalized());
    };
    if !entry.ships.iter().any(|s| s.template_path == template_path) {
        return (SelectionOutcome::Rejected, selection.normalized());
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
    let id = selection.scenario()?;
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
    let Some(id) = selection.scenario() else {
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

    // ── The shared parity table ─────────────────────────────────────────────
    //
    // `tests/fixtures/scenario-arbiter-parity.json` is one case table read by
    // BOTH implementations of this rule set: these two tests, and
    // `tests/client/scenario-arbiter-parity.test.js` driving
    // `gui/scenario-arbiter.js`. Until it existed the two arbiters were held
    // together by prose and by each having its own hand-written cases — so a
    // rule could move on one side with every test still green, which is exactly
    // how the empty-string handling below came apart.
    //
    // `include_str!` rather than a runtime read: the path is checked at compile
    // time, so the fixture cannot be moved or renamed without this failing to
    // build, and a green run cannot mean "the file was not found".

    const PARITY_FIXTURE: &str = include_str!("../../tests/fixtures/scenario-arbiter-parity.json");

    fn parity_fixture() -> serde_json::Value {
        serde_json::from_str(PARITY_FIXTURE).expect("the parity fixture is valid JSON")
    }

    fn parity_catalog(fixture: &serde_json::Value) -> ScenarioCatalog {
        ScenarioCatalog {
            scenarios: fixture["catalog"]
                .as_array()
                .expect("the fixture carries a `catalog` array")
                .iter()
                .map(|entry| ScenarioCatalogEntry {
                    id: entry["id"].as_str().expect("every entry has an id").into(),
                    world: entry["world"]
                        .as_str()
                        .expect("every entry names a world")
                        .into(),
                    label: entry["label"].as_str().map(str::to_string),
                    description: entry["description"].as_str().map(str::to_string),
                    ships: entry["ships"]
                        .as_array()
                        .expect("every entry has a ships array")
                        .iter()
                        .map(|s| AvailableShipEntry {
                            template_path: s["template_path"]
                                .as_str()
                                .expect("every hull has a template_path")
                                .into(),
                            label: s["label"].as_str().map(str::to_string),
                        })
                        .collect(),
                    origin: None,
                })
                .collect(),
        }
    }

    /// A selection as the table writes it: `null` is unlocked, and `""` is a
    /// deliberate case rather than a typo.
    fn parity_selection(value: &serde_json::Value) -> ScenarioSelection {
        ScenarioSelection {
            scenario_id: value["scenario_id"].as_str().map(str::to_string),
            template_path: value["template_path"].as_str().map(str::to_string),
        }
    }

    /// The JS's own outcome strings, which are what the table records.
    fn parity_outcome(outcome: SelectionOutcome) -> &'static str {
        match outcome {
            SelectionOutcome::Accepted => "accepted",
            SelectionOutcome::Ignored => "ignored",
            SelectionOutcome::Rejected => "rejected",
        }
    }

    #[test]
    fn scenario_arbiter_parity_outcomes() {
        let fixture = parity_fixture();
        let catalog = parity_catalog(&fixture);
        let cases = fixture["cases"]
            .as_array()
            .expect("the fixture carries a `cases` array");
        assert!(
            !cases.is_empty(),
            "a fixture nothing reads proves nothing about parity"
        );
        for case in cases {
            let name = case["name"].as_str().unwrap_or("<unnamed case>");
            let held = parity_selection(&case["selection"]);
            let argument = case["argument"]
                .as_str()
                .unwrap_or_else(|| panic!("{name}: every case names an argument"));
            let (outcome, after) = match case["call"].as_str() {
                Some("select_scenario") => select_scenario(&held, &catalog, argument),
                Some("select_player_ship") => select_player_ship(&held, &catalog, argument),
                other => panic!("{name}: unknown call {other:?}"),
            };
            assert_eq!(
                parity_outcome(outcome),
                case["outcome"].as_str().unwrap_or("<missing>"),
                "{name}: outcome"
            );
            assert_eq!(
                after,
                parity_selection(&case["selection_after"]),
                "{name}: the selection the call returns"
            );
        }
    }

    #[test]
    fn scenario_arbiter_parity_derived_answers() {
        let fixture = parity_fixture();
        let catalog = parity_catalog(&fixture);
        let rows = fixture["derived"]
            .as_array()
            .expect("the fixture carries a `derived` array");
        assert!(!rows.is_empty(), "a fixture nothing reads proves nothing");
        for row in rows {
            let name = row["name"].as_str().unwrap_or("<unnamed row>");
            let selection = parity_selection(&row["selection"]);
            assert_eq!(
                selection.is_complete(),
                row["is_complete"].as_bool().unwrap_or(false),
                "{name}: is_complete"
            );
            assert_eq!(
                world_path_for(&catalog, &selection),
                row["world_path"].as_str(),
                "{name}: world_path_for"
            );
            let expected: Vec<String> = row["curated_ships"]
                .as_array()
                .expect("every derived row lists curated_ships")
                .iter()
                .map(|v| v.as_str().expect("a hull path").to_string())
                .collect();
            assert_eq!(
                curated_ships_for(&catalog, &selection),
                expected,
                "{name}: curated_ships_for"
            );
        }
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
