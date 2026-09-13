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
}

pub fn catalog(files: BTreeMap<String, String>) -> TestCatalog {
    let source = Sources(files);
    TestCatalog {
        worlds: source
            .0
            .keys()
            .filter(|path| path.starts_with("assets/worlds/") && path.ends_with(".toml"))
            .cloned()
            .collect(),
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
    }
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
