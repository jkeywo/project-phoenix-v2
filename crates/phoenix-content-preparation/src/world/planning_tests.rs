use super::*;
use crate::world::load::{self, LoadPolicy, LoadRequest, MemoryReader};
use phoenix_sim_world::world::script::load::NoSiblingScripts;
const ROOT: &str = "tests/fixtures/maintenance-htn-world.toml";
const LIB: &str = "assets/worlds/domains/maintenance.toml";
fn source(path: &str) -> String {
    crate::content_fs::read_to_string(path).unwrap()
}
fn reader() -> MemoryReader {
    MemoryReader::new([(ROOT, source(ROOT)), (LIB, source(LIB))])
}
fn load_candidate(reader: &MemoryReader) -> load::LoadedWorld {
    load::load(
        LoadRequest::new(ROOT, reader, &NoSiblingScripts, LoadPolicy::Merge),
        &crate::entities::loader::FixtureTemplates,
        &crate::entities::include_resolve::FixtureFragments,
    )
    .unwrap()
}
#[test]
fn imported_library_uses_exact_reader_bytes_and_actual_offered_hull_capabilities() {
    let loader = crate::entities::loader::FixtureTemplates;
    let mut sources = reader();
    let loaded = load_candidate(&sources);
    assert_eq!(
        loaded
            .ledger
            .records
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        vec![ROOT, LIB]
    );
    assert_eq!(loaded.ledger.records[1].text, source(LIB));
    let inspect = |config: &crate::world::config::WorldConfig| {
        findings(&WorldSource::new(ROOT, &sources.0[ROOT], config), &loader)
    };
    assert!(inspect(&loaded.config).is_empty());
    let mut one = loaded.config.clone();
    let mut interior = crate::entities::loader::TemplateLoader::load_template(
        &loader,
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .interior
    .unwrap();
    interior.team_homes.truncate(1);
    let mut overrides: toml::Value = toml::from_str("[repair]\nrepair_team_count=1\n").unwrap();
    overrides
        .as_table_mut()
        .unwrap()
        .insert("interior".into(), toml::Value::try_from(&interior).unwrap());
    one.entities[0].overrides = Some(overrides);
    assert!(
        inspect(&one).is_empty(),
        "a compatible alternative is enough for this offered hull"
    );
    one.entities[0].overrides = Some(toml::from_str("[repair]\nrepair_team_count=0\n").unwrap());
    assert!(inspect(&one)
        .iter()
        .any(|row| row.category == "unsupported-maintenance-plan"));
    let original = source(LIB);
    sources.0.insert(
        LIB.into(),
        original.replace(
            "amount={source=\"literal\",value={kind=\"quantity\",value=4}}",
            "amount={source=\"literal\",value={kind=\"quantity\",value=3}}",
        ),
    );
    let changed = load_candidate(&sources);
    assert_ne!(
        loaded.ledger.records[1].text,
        changed.ledger.records[1].text
    );
    let graph = changed
        .config
        .compiled_planning
        .as_ref()
        .unwrap()
        .choose("actual", 2, |_| Ok(()))
        .unwrap()
        .graph;
    assert!(graph.leaves.values().any(|row| matches!(
        row.request,
        phoenix_sim_world::world::activities::ActivityRequest::Delivery { amount: 3, .. }
    )));
}
#[test]
fn unresolved_or_unexported_domain_refuses_before_activation_while_inspect_stays_parse_only() {
    let mut sources = reader();
    sources.0.remove(LIB);
    let run = |sources: &MemoryReader, policy| {
        load::load(
            LoadRequest::new(ROOT, sources, &NoSiblingScripts, policy),
            &crate::entities::loader::FixtureTemplates,
            &crate::entities::include_resolve::FixtureFragments,
        )
    };
    let inspected = run(&sources, LoadPolicy::Inspect).unwrap();
    assert!(inspected.ledger.records.is_empty());
    assert!(inspected.config.compiled_planning.is_none());
    let error = run(&sources, LoadPolicy::Activate).unwrap_err().to_string();
    assert!(error.contains(LIB));
    sources.0.insert(
        LIB.into(),
        source(LIB).replace("exports=[\"maintain\"]", "exports=[\"restore-room\"]"),
    );
    assert!(run(&sources, LoadPolicy::Activate).is_err());
}
