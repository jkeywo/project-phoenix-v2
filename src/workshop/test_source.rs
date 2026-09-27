//! Runtime-owned Test selection over an already admitted immutable source map.
//! Native and browser adapters share these composed hull/world checks.
use super::{test_protocol::TestSelection, Sources, WorkshopValidation};
use crate::entities::{include_resolve::canonical_template_path, loader::TemplateLoader};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct TestCatalog {
    pub worlds: Vec<String>,
    pub ships: Vec<String>,
    /// Authored choices for the selected world's controlled ship slot.
    pub slots: BTreeMap<String, Vec<TestSlotOffer>>,
    /// Exact authored child layers loaded with each selectable root. The root
    /// itself is represented by the breakpoint's absent `layer`, never by a
    /// `WorldLayerMap` key.
    pub layers: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct TestSlotOffer {
    pub id: String,
    pub label: Option<String>,
    pub ships: Vec<String>,
    pub default_ship: String,
    pub unclaimed: crate::world::config::UnclaimedSlotPolicy,
}

pub fn catalog(files: BTreeMap<String, String>) -> TestCatalog {
    let source = Sources(files);
    let worlds = source
        .0
        .keys()
        .filter(|path| path.starts_with("assets/worlds/") && path.ends_with(".toml"))
        .cloned()
        .collect::<Vec<_>>();
    let layers = worlds
        .iter()
        .map(|world| (world.clone(), breakpoint_layers(&source, world)))
        .collect();
    let slots = worlds
        .iter()
        .map(|world| {
            let offered = source
                .0
                .get(world)
                .and_then(|text| crate::world::config::parse_world(text).ok())
                .map(|config| {
                    config
                        .ship_slots
                        .into_iter()
                        .map(|slot| TestSlotOffer {
                            id: slot.id,
                            label: slot.label,
                            ships: slot
                                .ships
                                .into_iter()
                                .map(|ship| ship.template_path)
                                .collect(),
                            default_ship: slot.default_ship,
                            unclaimed: slot.unclaimed,
                        })
                        .collect()
                })
                .unwrap_or_default();
            (world.clone(), offered)
        })
        .collect();
    TestCatalog {
        worlds,
        slots,
        ships: source
            .0
            .keys()
            .filter(|path| {
                path.starts_with("assets/entities/")
                    && path.ends_with(".toml")
                    && source
                        .load_template(path)
                        .is_some_and(|hull| hull.class.is_some() && hull.ship_config.is_some())
            })
            .cloned()
            .collect(),
        layers,
    }
}

/// The layers which the ordinary world loader installs beside `root`.
pub(super) fn breakpoint_layers(source: &Sources, root: &str) -> Vec<String> {
    crate::world::load::load(crate::world::load::LoadRequest::new(
        root,
        source,
        source,
        crate::world::load::LoadPolicy::Inspect,
    ))
    .map(|loaded| loaded.config.extra_worlds)
    .unwrap_or_default()
}

pub fn validate_selection(
    files: BTreeMap<String, String>,
    selection: &TestSelection,
) -> WorkshopValidation {
    let source = Sources(files);
    let mut report = WorkshopValidation::default();
    let exact_path = |path: &str, directory: &str| {
        path.starts_with(directory)
            && path.ends_with(".toml")
            && canonical_template_path(path) == path
            && source.0.contains_key(path)
    };
    if !exact_path(&selection.world, "assets/worlds/") {
        report.error(
            "runtime-world-invalid",
            &selection.world,
            "Select an authored world".into(),
        );
    } else {
        super::validate_world(&selection.world, &source, &mut report);
        // The loader accepts a flat list one level deep, so a duplicate,
        // cyclic or disallowed child and a dangling scripted load are
        // invisible to it; a Test runs the exact candidate, so it clears
        // the composition rules save and export apply (issue #1475).
        report.extend_workshop(super::composition::selection_findings(
            &source.0,
            &selection.world,
        ));
        if let Some(config) = source
            .0
            .get(&selection.world)
            .and_then(|text| crate::world::config::parse_world(text).ok())
        {
            if config.ship_slots.is_empty() {
                if selection.slot.is_some() {
                    report.error(
                        "runtime-slot-invalid",
                        &selection.world,
                        "Legacy world has no authored ship slot to select".into(),
                    );
                }
            } else if let Some(slot) = config
                .ship_slots
                .iter()
                .find(|slot| selection.slot.as_deref() == Some(slot.id.as_str()))
            {
                if !slot
                    .ships
                    .iter()
                    .any(|ship| ship.template_path == selection.ship)
                {
                    report.error(
                        "runtime-slot-hull-invalid",
                        &selection.world,
                        format!("Selected hull is not offered by ship slot {:?}", slot.id),
                    );
                }
            } else {
                report.error(
                    "runtime-slot-invalid",
                    &selection.world,
                    "Select an authored ship slot".into(),
                );
            }
        }
    }
    if !exact_path(&selection.ship, "assets/entities/")
        || !source
            .load_template(&selection.ship)
            .is_some_and(|hull| hull.class.is_some() && hull.ship_config.is_some())
    {
        report.error(
            "runtime-template-invalid",
            &selection.ship,
            "Selected Test hull requires a complete composed runtime template with a ship configuration".into(),
        );
    }
    report.accepted = !report
        .findings
        .iter()
        .any(|finding| finding.severity == "error");
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "assets/worlds/root.toml";
    const CHILD: &str = "assets/worlds/child.toml";
    const HULL: &str = "assets/entities/hull.toml";
    const HULL_SOURCE: &str = "class='lancer'\nname='Test hull'\n\
[[station]]\nid='captain'\nname='Captain'\ndescription='Test station'\nrank='captain'\n\
[[system]]\nid='boost'\nkind='helm_boost'\nstation='captain'\n";

    #[test]
    fn authored_test_slot_offers_and_selection_use_the_runtime_hull_allowlist() {
        const OTHER: &str = "assets/entities/other.toml";
        let world = format!(
            "[global]\ntitle = 'Slots'\n\
[[ship_slot]]\nid = 'lead'\ndefault_ship = '{HULL}'\nunclaimed = 'backfill'\n\
[[ship_slot.ships]]\ntemplate_path = '{HULL}'\n\
[[ship_slot]]\nid = 'wing'\ndefault_ship = '{OTHER}'\nunclaimed = 'absent'\n\
[[ship_slot.ships]]\ntemplate_path = '{OTHER}'\n\
[[entity]]\nid = 'lead-row'\ntemplate_path = '{HULL}'\nspawn_on = 'game_start'\n\
[[entity]]\nid = 'wing-row'\ntemplate_path = '{OTHER}'\nspawn_on = 'game_start'\n"
        );
        let files = BTreeMap::from([
            (ROOT.into(), world),
            (HULL.into(), format!("tags=['ship']\n{HULL_SOURCE}")),
            (OTHER.into(), format!("tags=['ship']\n{HULL_SOURCE}")),
        ]);
        let offers = catalog(files.clone());
        assert_eq!(offers.slots[ROOT].len(), 2);
        assert_eq!(offers.slots[ROOT][1].id, "wing");
        assert_eq!(offers.slots[ROOT][1].ships, [OTHER]);

        let selected = TestSelection {
            world: ROOT.into(),
            slot: Some("wing".into()),
            ship: OTHER.into(),
            seed: 7,
        };
        let accepted = validate_selection(files.clone(), &selected);
        assert!(accepted.accepted, "{:?}", accepted.findings);
        let wrong_hull = TestSelection {
            ship: HULL.into(),
            ..selected.clone()
        };
        assert!(validate_selection(files.clone(), &wrong_hull)
            .findings
            .iter()
            .any(|finding| finding.category == "runtime-slot-hull-invalid"));
        let no_slot = TestSelection {
            slot: None,
            ..selected
        };
        assert!(validate_selection(files, &no_slot)
            .findings
            .iter()
            .any(|finding| finding.category == "runtime-slot-invalid"));
    }

    /// A Test runs the exact unsaved composition (issue #1475): a child the
    /// draft declares and carries is accepted although nothing beneath the
    /// draft knows it, and the same root is refused once the child is gone.
    #[test]
    fn a_draft_declared_child_present_only_in_the_candidate_runs_and_a_missing_one_refuses() {
        let mut files = BTreeMap::from([
            (
                ROOT.to_owned(),
                "extra_worlds = [\"assets/worlds/child.toml\"]\n[global]\ntitle = \"Root\"\n"
                    .to_owned(),
            ),
            (
                CHILD.to_owned(),
                "[global]\ntitle = \"Child\"\n[[entity]]\nname = \"Scout\"\ntemplate_path = \"assets/entities/hull.toml\"\n"
                    .to_owned(),
            ),
            (HULL.to_owned(), HULL_SOURCE.to_owned()),
        ]);
        let selection = TestSelection {
            world: ROOT.into(),
            slot: None,
            ship: HULL.into(),
            seed: 7,
        };
        let accepted = validate_selection(files.clone(), &selection);
        assert!(accepted.accepted, "{:?}", accepted.findings);
        files.remove(CHILD);
        let refused = validate_selection(files, &selection);
        assert!(!refused.accepted);
        assert!(
            refused
                .findings
                .iter()
                .any(|finding| finding.severity == "error"
                    && finding.file == CHILD
                    && finding.category == "runtime-world-invalid"),
            "{:?}",
            refused.findings
        );
        // The composition rule names the entry's own line beside the
        // loader's error, so the author is sent to the reference.
        assert!(
            refused.findings.iter().any(|finding| finding.file == ROOT
                && finding.category == "extra-worlds-missing"
                && finding.line == Some(1)),
            "{:?}",
            refused.findings
        );
    }

    /// The loader reads `extra_worlds` as a flat list, so it would start a
    /// Test over a root whose composition save and export refuse. The Test
    /// gate clears the composition rules of the selected root and its
    /// children — and only those: a broken world the selection does not
    /// compose is Check's business, not a bar to this Test.
    #[test]
    fn a_test_is_refused_for_a_composition_rule_the_selected_root_breaks_and_not_for_another_world()
    {
        let files = BTreeMap::from([
            (
                ROOT.to_owned(),
                "extra_worlds = [\n    \"assets/worlds/child.toml\",\n    \"assets/worlds/child.toml\",\n]\n[global]\ntitle = \"Root\"\n"
                    .to_owned(),
            ),
            (
                CHILD.to_owned(),
                "extra_worlds = [\"assets/worlds/root.toml\"]\n[global]\ntitle = \"Child\"\n"
                    .to_owned(),
            ),
            (
                "assets/worlds/elsewhere.toml".to_owned(),
                "[global]\n[[action]]\ntype = \"load_world\"\npath = \"assets/worlds/nope.toml\"\n"
                    .to_owned(),
            ),
            (HULL.to_owned(), HULL_SOURCE.to_owned()),
        ]);
        let selection = TestSelection {
            world: ROOT.into(),
            slot: None,
            ship: HULL.into(),
            seed: 7,
        };
        let refused = validate_selection(files.clone(), &selection);
        assert!(!refused.accepted);
        let located: Vec<(&str, &str, Option<usize>)> = refused
            .findings
            .iter()
            .map(|finding| {
                (
                    finding.category.as_str(),
                    finding.file.as_str(),
                    finding.line,
                )
            })
            .collect();
        assert!(
            located.contains(&("extra-worlds-duplicate", ROOT, Some(3))),
            "{located:?}"
        );
        assert!(
            located.contains(&("extra-worlds-cycle", ROOT, Some(2))),
            "{located:?}"
        );
        assert!(
            located.contains(&("extra-worlds-cycle", CHILD, Some(1))),
            "{located:?}"
        );
        assert!(
            !located
                .iter()
                .any(|(_, file, _)| *file == "assets/worlds/elsewhere.toml"),
            "{located:?}"
        );
        let elsewhere = validate_selection(
            files,
            &TestSelection {
                world: "assets/worlds/elsewhere.toml".into(),
                slot: None,
                ship: HULL.into(),
                seed: 7,
            },
        );
        assert!(!elsewhere.accepted);
        assert!(
            elsewhere
                .findings
                .iter()
                .any(|finding| finding.category == "world-missing-load-reference"
                    && finding.file == "assets/worlds/elsewhere.toml"
                    && finding.line == Some(4)),
            "{:?}",
            elsewhere.findings
        );
        assert!(
            !elsewhere
                .findings
                .iter()
                .any(|finding| finding.file == ROOT),
            "{:?}",
            elsewhere.findings
        );
    }
}
