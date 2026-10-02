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
        slots: Vec::new(),
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
    let (outcome, selection) = select_scenario(&ScenarioSelection::default(), &catalog(), "patrol");
    assert_eq!(outcome, SelectionOutcome::Accepted);
    assert_eq!(selection.scenario_id.as_deref(), Some("patrol"));
    assert!(selection.template_path.is_none());
}

#[test]
fn a_later_scenario_request_is_ignored_rather_than_refused() {
    let locked = ScenarioSelection {
        scenario_id: Some("patrol".into()),
        slot_id: None,
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
        slot_id: None,
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
        slot_id: None,
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
        slot_id: None,
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
        slot_id: None,
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
                slots: Vec::new(),
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
        slot_id: value["slot_id"].as_str().map(str::to_string),
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
    let wire = crate::delivery::payload::catalog_payload(&catalog());
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
