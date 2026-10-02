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
#[path = "test_source_tests.rs"]
mod tests;
