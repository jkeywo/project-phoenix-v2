use super::*;
use crate::workshop::WorkshopDependencyPack;

const HULL: &str = "assets/entities/hull.toml";
const CORE: &str = "assets/entities/fragments/core.toml";
const EXTRA: &str = "assets/entities/fragments/extra.toml";
const CYCLE: &str = "assets/entities/fragments/cycle.toml";
/// A fragment the base set supplies, so an include and a choice carry a
/// read-only origin.
const SHARED: &str = "assets/entities/fragments/shared.toml";
/// A fragment an installed pack supplies.
const PACKED: &str = "assets/entities/fragments/packed.toml";

/// The draft fragment the hull composes: a hull table, a station, one keyed
/// system and a tag list.
fn core() -> String {
    [
        "# The shared core",
        "tags = [\"core\"]",
        "",
        "[hull]",
        "hull_integrity = 120.0",
        "",
        "[[station]]",
        "id = \"captain\"",
        "name = \"Captain\"",
        "description = \"The captain's station\"",
        "rank = \"captain\"",
        "",
        "[[system]]",
        "id = \"boost\"",
        "kind = \"helm_boost\"",
        "station = \"captain\"",
        "",
    ]
    .join("\n")
}

/// The hull: two includes (one from the draft, one from the base set), a
/// leading comment and a trailing comment on a value line.
fn hull() -> String {
    [
        "# The hull itself",
        "includes = [",
        "    \"fragments/core.toml\",",
        "    \"fragments/shared.toml\",",
        "]",
        "class = \"lancer\" # keep this tail",
        "name = \"Test hull\"",
        "",
    ]
    .join("\n")
}

fn draft() -> BTreeMap<String, String> {
    BTreeMap::from([
        (HULL.to_owned(), hull()),
        (CORE.to_owned(), core()),
        (
            EXTRA.to_owned(),
            "[reference_grid]\nplane_y = -1.0\n".to_owned(),
        ),
        // A fragment that includes the hull: adding it to the hull would close
        // a cycle, so it must never be offered as a choice.
        (
            CYCLE.to_owned(),
            "includes = [\"../hull.toml\"]\n".to_owned(),
        ),
    ])
}

fn dependencies() -> WorkshopDependencies {
    WorkshopDependencies {
        base_files: BTreeMap::from([(
            SHARED.to_owned(),
            "[repair]\nrepair_rate_hp_per_sec = 2.0\n".to_owned(),
        )]),
        base_assets: BTreeMap::new(),
        packs: vec![WorkshopDependencyPack {
            id: "aurora".to_owned(),
            manifest_toml: "[pack]\nid = \"aurora\"\n".to_owned(),
            files: BTreeMap::from([(PACKED.to_owned(), "[scan]\n".to_owned())]),
            assets: BTreeMap::new(),
        }],
    }
}

fn request(files: &BTreeMap<String, String>, path: &str, edits: Vec<Edit>) -> EntityEditRequest {
    EntityEditRequest {
        document_path: path.to_owned(),
        expected_source: files[path].clone(),
        edits,
    }
}

fn include_entry(value: &str) -> String {
    format!("\"{value}\"")
}

fn categories(findings: &[WorkshopFinding], file: &str) -> Vec<(String, Option<usize>)> {
    findings
        .iter()
        .filter(|finding| finding.file == file)
        .map(|finding| (finding.category.clone(), finding.line))
        .collect()
}

// ── The runtime's own vocabulary ──────────────────────────────────────────────

/// The ratchet on D2: the component list is SERDE's, so a component added to
/// `EntityConfig` appears in the Workshop with no edit in `entity.rs`. This
/// test is the only place the names are written down, and it fails when the
/// struct changes — which is the point.
#[test]
fn supported_components_are_serdes_own_field_list() {
    assert_eq!(
        supported_components(),
        [
            "name",
            "display_name",
            "tags",
            "hull",
            "collider",
            "appearance",
            "helm_console",
            "helm_capability",
            "weapons_console",
            "engineering_console",
            "captain_console",
            "comms_console",
            "power",
            "sensors_console",
            "navigation_console",
            "shields_console",
            "torpedoes",
            "repair",
            "audio",
            "comms",
            "asteroid_field",
            "shape",
            "effects",
            "infrastructure",
            "scan",
            "debris",
            "tractor",
            "held_response",
            "dock",
            "umbilical",
            "security",
            "security_target",
            "transporter",
            "civilian_rescue",
            "demolition_target",
            "reference_grid",
            "civilian",
            "faction",
            "behaviour",
            "ai_profile",
            "lod_bubble",
            "radar_appearance",
            "target",
            "mesh",
            "star",
            "planet",
            "class",
            "hull_id",
            "power_rating",
            "css",
            "mass",
            "light",
            "cinematic_camera",
        ]
        .map(str::to_owned)
    );
    // The list is read out of the error, so the error has to keep its shape:
    // an empty list would silently offer nothing at all.
    assert!(!supported_components().is_empty());
    // What serde does NOT name, and why nothing refuses it: `includes` is the
    // resolver's own key, and the ship blocks are consumed by
    // `EntityConfig::from_toml` before serde sees the document. Authoring
    // those structurally is #1481's; the edit-time rule asks the runtime
    // whether the document parses rather than consulting this list.
    for consumed in [
        "includes",
        "station",
        "system",
        "power_groups",
        "shield_arc",
    ] {
        assert!(
            !supported_components().iter().any(|key| key == consumed),
            "{consumed} is not one of serde's fields"
        );
    }
}

/// Every skeleton is the runtime's own answer, so every skeleton has to be a
/// document the runtime reads back. The key list is pinned so a component that
/// gains or loses a `Default` is a decision here rather than a silent change
/// in what the panel offers.
#[test]
fn every_component_skeleton_is_a_document_the_runtime_accepts() {
    assert_eq!(
        component_skeletons().keys().cloned().collect::<Vec<_>>(),
        [
            "audio",
            "captain_console",
            "civilian",
            "comms_console",
            "debris",
            "effects",
            "engineering_console",
            "helm_capability",
            "helm_console",
            "hull",
            "infrastructure",
            "navigation_console",
            "reference_grid",
            "repair",
            "sensors_console",
            "shields_console",
            "star",
            "target",
        ]
        .map(str::to_owned)
    );
    for (key, skeleton) in component_skeletons() {
        assert!(
            supported_components().iter().any(|field| field == key),
            "{key} is not a component EntityConfig knows"
        );
        EntityConfig::from_toml(&format!("{key} = {skeleton}\n"))
            .unwrap_or_else(|error| panic!("{key} = {skeleton} is not readable: {error}"));
    }
    // A component with no skeleton is LISTED honestly rather than missing, so
    // the panel can say why it offers no Add.
    let catalog = catalog(&draft(), &dependencies(), HULL);
    let state = |key: &str| {
        catalog
            .components
            .iter()
            .find(|component| component.key == key)
            .map(|component| component.skeleton)
    };
    assert_eq!(state("hull"), Some(true));
    assert_eq!(state("collider"), Some(false));
    assert_eq!(state("mass"), Some(false));
    assert_eq!(catalog.components.len(), supported_components().len());

    // The panel's Add writes this default as ONE exact-source value, so the
    // flag alone cannot serve it: the TEXT travels with the flag, exactly when
    // the flag is set, and every one of them is a `put` the edit owner accepts.
    for component in &catalog.components {
        assert_eq!(
            component.skeleton,
            component.skeleton_source.is_some(),
            "{} claims skeleton {} and carries {:?}",
            component.key,
            component.skeleton,
            component.skeleton_source
        );
        let Some(source) = component.skeleton_source.as_deref() else {
            continue;
        };
        assert_eq!(Some(source), component_skeleton(&component.key));
        let blank = "name = \"probe\"\n";
        let written = document::edit(
            blank,
            &EditRequest {
                document_path: HULL.to_owned(),
                expected_source: blank.to_owned(),
                edits: vec![Edit::Put {
                    path: vec![Segment::Key(component.key.clone())],
                    value_source: source.to_owned(),
                }],
            },
        )
        .unwrap_or_else(|error| {
            panic!(
                "{} = {source} is not a value an exact-source put can write: {error}",
                component.key
            )
        });
        assert!(
            written.contains(&format!("{} = {source}", component.key)),
            "{} = {source} did not land verbatim: {written}",
            component.key
        );
    }
}

// ── Catalog ──────────────────────────────────────────────────────────────────

#[test]
fn the_catalog_reads_includes_merge_order_and_origins() {
    let catalog = catalog(&draft(), &dependencies(), HULL);
    assert_eq!(catalog.path, HULL);
    assert_eq!(catalog.origin, "draft");
    assert!(catalog.resolvable, "{:?}", catalog.error);
    assert_eq!(catalog.error, None);
    assert_eq!(
        catalog
            .includes
            .iter()
            .map(|entry| (
                entry.index,
                entry.authored.as_str(),
                entry.canonical.as_str(),
                entry.line,
                entry.origin.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            (0, "fragments/core.toml", CORE, 3, Some("draft")),
            (1, "fragments/shared.toml", SHARED, 4, Some("base")),
        ]
    );
    // Merge order is the resolver's: each fragment, then the declaring
    // template last, so the includer wins.
    assert_eq!(catalog.sources, vec![CORE, SHARED, HULL]);
    // Every other entity member that would not close a cycle, and nothing
    // else: not the hull itself and not the fragment that includes it.
    assert_eq!(
        catalog
            .fragment_choices
            .iter()
            .map(|choice| (choice.path.as_str(), choice.origin.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (CORE, "draft"),
            (EXTRA, "draft"),
            (SHARED, "base"),
            (PACKED, "pack:aurora"),
        ]
    );
    assert!(catalog.findings.is_empty(), "{:?}", catalog.findings);
}

#[test]
fn components_say_local_inherited_or_absent_with_the_owner() {
    let catalog = catalog(&draft(), &dependencies(), HULL);
    let component = |key: &str| component(&catalog, key);
    let hull = component("hull");
    assert_eq!(
        (hull.local, hull.local_line, hull.inherited_from.as_deref()),
        (false, None, Some(CORE))
    );
    let repair = component("repair");
    assert_eq!(repair.inherited_from.as_deref(), Some(SHARED));
    let name = component("name");
    assert_eq!((name.local, name.local_line), (true, Some(7)));
    assert_eq!(name.inherited_from, None);
    let absent = component("planet");
    assert_eq!(
        (
            absent.local,
            absent.local_line,
            absent.inherited_from.clone()
        ),
        (false, None, None)
    );
}

fn component<'a>(catalog: &'a EntityComposition, key: &str) -> &'a ComponentView {
    catalog
        .components
        .iter()
        .find(|component| component.key == key)
        .unwrap_or_else(|| panic!("{key} is listed"))
}

fn remove_component(key: &str) -> Vec<Edit> {
    vec![Edit::Remove {
        path: vec![Segment::Key(key.to_owned())],
    }]
}

/// A component the local document authors and a fragment ALSO authors is BOTH
/// local and inherited — the central case of composition, and the shape 38
/// (component, fragment) pairs of the shipped composed hulls are in. The catalog
/// says both, and dropping the local override is an ordinary edit rather than a
/// refusal: there IS local text to remove, and the row said which fragment the
/// value reverts to.
#[test]
fn a_component_the_template_partly_overrides_is_local_and_inherited_and_droppable() {
    let mut files = draft();
    // One key each: core() authors `hull.hull_integrity` and the hull authors a
    // different key of the same component.
    files.insert(
        HULL.to_owned(),
        format!("{}\n[hull]\nsystem_hull = []\n", hull().trim_end()),
    );
    let catalog = catalog(&files, &dependencies(), HULL);
    let hull_component = component(&catalog, "hull");
    assert_eq!(
        (
            hull_component.local,
            hull_component.local_line,
            hull_component.inherited_from.as_deref()
        ),
        (true, Some(8), Some(CORE))
    );
    // The composed value carries both halves, which is what makes the row's
    // Remove a legitimate control rather than a dead one.
    let integrity = catalog
        .fields
        .iter()
        .find(|field| field.address == "hull.hull_integrity")
        .unwrap();
    assert_eq!((integrity.local, integrity.source.as_str()), (false, CORE));

    let removed = compose(
        &files,
        &dependencies(),
        &request(&files, HULL, remove_component("hull")),
    )
    .expect("dropping a local override is not a refusal");
    assert_eq!(removed, hull());
}

/// A component whose every inherited leaf the local document SHADOWS is the
/// opposite blindness of the same reading: provenance records only winners, so
/// no leaf beneath it has a foreign origin and the component read as purely
/// local — while the inherited table is still there underneath, ready to be
/// composed back the moment the local text goes. The catalog names it, so the
/// removal is announced instead of silently undone.
#[test]
fn a_component_whose_every_inherited_leaf_is_shadowed_still_names_its_fragment() {
    let mut files = draft();
    // `hull_integrity` is the only key core() authors under `[hull]`, so the
    // hull wins every leaf of the component.
    files.insert(
        HULL.to_owned(),
        format!("{}\n[hull]\nhull_integrity = 99.0\n", hull().trim_end()),
    );
    let shadowed = catalog(&files, &dependencies(), HULL);
    let hull_component = component(&shadowed, "hull");
    assert_eq!(
        (
            hull_component.local,
            hull_component.inherited_from.as_deref()
        ),
        (true, Some(CORE))
    );
    assert!(shadowed
        .fields
        .iter()
        .all(|field| field.address != "hull.hull_integrity" || field.local));

    // Removing the local text is allowed — and what it does is exactly what the
    // row said: the fragment's copy composes again.
    let removed = compose(
        &files,
        &dependencies(),
        &request(&files, HULL, remove_component("hull")),
    )
    .unwrap();
    let mut after = files.clone();
    after.insert(HULL.to_owned(), removed);
    let reverted = catalog(&after, &dependencies(), HULL);
    let hull_component = component(&reverted, "hull");
    assert_eq!(
        (
            hull_component.local,
            hull_component.inherited_from.as_deref()
        ),
        (false, Some(CORE))
    );
    assert_eq!(
        reverted
            .fields
            .iter()
            .find(|field| field.address == "hull.hull_integrity")
            .map(|field| (field.local, field.value_source.as_str())),
        Some((false, "120.0"))
    );
}

/// The fields are provenance's, not a second merge: every address, owner and
/// chain the catalog reports is the one `resolve_template` recorded, and a
/// local value carries this document's own exact span and line.
#[test]
fn fields_are_provenance_read_back_with_exact_local_source() {
    let files = draft();
    let mut sources = dependencies().base_files.clone();
    sources.extend(dependencies().packs[0].files.clone());
    sources.extend(files.clone());
    let resolved = resolve_template(HULL, &sources).unwrap();
    let catalog = catalog(&files, &dependencies(), HULL);

    assert_eq!(
        catalog
            .fields
            .iter()
            .map(|field| field.address.clone())
            .collect::<Vec<_>>(),
        resolved
            .provenance
            .fields()
            .map(|(address, _)| address.clone())
            .collect::<Vec<_>>()
    );
    for field in &catalog.fields {
        let origin = resolved.provenance.origin(&field.address).unwrap();
        assert_eq!(
            (&field.source, &field.chain),
            (&origin.source, &origin.chain)
        );
        assert_eq!(field.local, origin.source == HULL);
        match field.line {
            Some(line) => {
                assert!(field.local, "{} is not local", field.address);
                let text = hull();
                let on_line = text.lines().nth(line - 1).unwrap();
                assert!(
                    on_line.contains(&field.value_source),
                    "{} is not the source at line {line}: {on_line}",
                    field.value_source
                );
            }
            None => assert!(!field.local, "{} has no line", field.address),
        }
    }
    let field = |address: &str| {
        catalog
            .fields
            .iter()
            .find(|field| field.address == address)
            .unwrap_or_else(|| panic!("{address} is a field"))
    };
    // An inherited value is the resolved one serialised, with its owner and
    // the chain that reached it, and no line in this document.
    let integrity = field("hull.hull_integrity");
    assert_eq!(
        (
            integrity.local,
            integrity.source.as_str(),
            integrity.value_source.as_str(),
            integrity.line,
            integrity.kind.as_deref()
        ),
        (false, CORE, "120.0", None, Some("float"))
    );
    assert_eq!(integrity.chain, vec![HULL.to_owned(), CORE.to_owned()]);
    // A keyed array entry is addressed by the key the merge reconciles it by.
    assert_eq!(field("system[id=boost].kind").source, CORE);
    // A local value is the exact span text, quotes and all.
    let class = field("class");
    assert_eq!(
        (class.local, class.value_source.as_str(), class.line),
        (true, "\"lancer\"", Some(6))
    );
    // An inherited ARRAY is one leaf, because the merge reconciles it whole.
    assert_eq!(field("tags").value_source, "[\"core\"]");
}

#[test]
fn a_path_that_is_in_neither_the_draft_nor_its_dependencies_says_so() {
    let catalog = catalog(&draft(), &dependencies(), "assets/entities/ghost.toml");
    assert!(!catalog.resolvable);
    assert!(
        catalog
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("unknown-document: ")),
        "{:?}",
        catalog.error
    );
    assert_eq!(catalog.origin, "");
    // The vocabulary is still served, so the panel can render its sections.
    assert_eq!(catalog.supported_components, supported_components());
}

#[test]
fn an_unresolvable_closure_still_shows_what_the_file_itself_authors() {
    let mut files = draft();
    files.insert(
        HULL.to_owned(),
        "includes = [\"fragments/gone.toml\"]\nname = \"Test hull\"\n".to_owned(),
    );
    let catalog = catalog(&files, &dependencies(), HULL);
    assert!(!catalog.resolvable);
    assert!(
        catalog
            .error
            .as_deref()
            .is_some_and(|error| error.contains("include-missing")),
        "{:?}",
        catalog.error
    );
    assert_eq!(catalog.includes[0].origin, None);
    let name = catalog
        .components
        .iter()
        .find(|component| component.key == "name")
        .unwrap();
    assert_eq!((name.local, name.local_line), (true, Some(2)));
}

// ── Include edits (criterion 1 and 3) ────────────────────────────────────────

/// Adding, removing and reordering an include is one exact-source edit: every
/// other byte of the template — the comments, the key order, the trailing
/// comment — and every byte of the included files survive.
#[test]
fn include_add_remove_and_reorder_keep_every_other_byte() {
    let files = draft();
    let added = compose(
        &files,
        &dependencies(),
        &request(
            &files,
            HULL,
            vec![Edit::Insert {
                path: vec![Segment::Key(INCLUDES_KEY.to_owned())],
                index: 1,
                value_source: include_entry("fragments/extra.toml"),
            }],
        ),
    )
    .unwrap();
    assert_eq!(
        added,
        [
            "# The hull itself",
            "includes = [",
            "    \"fragments/core.toml\",",
            "    \"fragments/extra.toml\",",
            "    \"fragments/shared.toml\",",
            "]",
            "class = \"lancer\" # keep this tail",
            "name = \"Test hull\"",
            "",
        ]
        .join("\n")
    );

    let removed = compose(
        &files,
        &dependencies(),
        &request(
            &files,
            HULL,
            vec![Edit::Remove {
                path: vec![Segment::Key(INCLUDES_KEY.to_owned()), Segment::Index(1)],
            }],
        ),
    )
    .unwrap();
    assert_eq!(
        removed,
        [
            "# The hull itself",
            "includes = [",
            "    \"fragments/core.toml\",",
            "]",
            "class = \"lancer\" # keep this tail",
            "name = \"Test hull\"",
            "",
        ]
        .join("\n")
    );

    // One press, one history entry: a reorder is a remove and an insert in
    // ONE request.
    let reordered = compose(
        &files,
        &dependencies(),
        &request(
            &files,
            HULL,
            vec![
                Edit::Remove {
                    path: vec![Segment::Key(INCLUDES_KEY.to_owned()), Segment::Index(0)],
                },
                Edit::Insert {
                    path: vec![Segment::Key(INCLUDES_KEY.to_owned())],
                    index: 1,
                    value_source: include_entry("fragments/core.toml"),
                },
            ],
        ),
    )
    .unwrap();
    assert_eq!(
        reordered,
        [
            "# The hull itself",
            "includes = [",
            "    \"fragments/shared.toml\",",
            "    \"fragments/core.toml\",",
            "]",
            "class = \"lancer\" # keep this tail",
            "name = \"Test hull\"",
            "",
        ]
        .join("\n")
    );
    // Nothing but the edited member was ever handed back, and the draft the
    // caller holds is untouched.
    assert_eq!(files, draft());
}

/// The reorder the PANEL emits, through the exact-source owner: `planIncludeEdits`
/// expresses a swap as a `set` on each slot so every entry's comments and blank
/// lines stay with their positions, and nothing else exercised that shape — the
/// test above sends a remove and an insert, which is not the route the UI takes.
#[test]
fn the_panels_own_reorder_sets_each_slot_and_keeps_every_comment_in_place() {
    let mut files = draft();
    let source = [
        "# The hull itself",
        "includes = [",
        "    \"fragments/core.toml\", # the core",
        "",
        "    \"fragments/shared.toml\", # the shared one",
        "]",
        "class = \"lancer\" # keep this tail",
        "",
    ]
    .join("\n");
    files.insert(HULL.to_owned(), source.clone());
    let reordered = compose(
        &files,
        &dependencies(),
        &request(
            &files,
            HULL,
            vec![
                Edit::Set {
                    path: vec![Segment::Key(INCLUDES_KEY.to_owned()), Segment::Index(0)],
                    value_source: include_entry("fragments/shared.toml"),
                },
                Edit::Set {
                    path: vec![Segment::Key(INCLUDES_KEY.to_owned()), Segment::Index(1)],
                    value_source: include_entry("fragments/core.toml"),
                },
            ],
        ),
    )
    .unwrap();
    // Only the two quoted paths moved: the per-entry comments, the blank line
    // between them, the trailing comma and the tail comment are where they were.
    assert_eq!(
        reordered,
        source
            .replace("\"fragments/core.toml\"", "\"fragments/PLACE.toml\"")
            .replace("\"fragments/shared.toml\"", "\"fragments/core.toml\"")
            .replace("\"fragments/PLACE.toml\"", "\"fragments/shared.toml\"")
    );
    let mut after = files.clone();
    after.insert(HULL.to_owned(), reordered);
    // …and the merge order the resolver reads followed the swap.
    let catalog = catalog(&after, &dependencies(), HULL);
    assert_eq!(catalog.sources, vec![SHARED, CORE, HULL]);
}

/// A template carrying an unknown top-level key is still editable, and the key
/// survives byte-for-byte. The rule that allows it is the same one that lets a
/// fragment be edited: a violation the member ALREADY had is a finding, not a
/// refusal, so an author can repair a hand-broken draft one edit at a time.
#[test]
fn an_unknown_key_and_crlf_survive_an_include_edit() {
    let mut files = draft();
    let source = "# kept\r\nincludes = [\r\n    \"fragments/core.toml\",\r\n]\r\nnotes = \"kept\"\r\nname = \"Test hull\"\r\n";
    files.insert(HULL.to_owned(), source.to_owned());
    assert!(
        EntityConfig::from_toml(source).is_err(),
        "the fixture has to be a document the runtime already refuses"
    );
    let edited = compose(
        &files,
        &dependencies(),
        &request(
            &files,
            HULL,
            vec![Edit::Insert {
                path: vec![Segment::Key(INCLUDES_KEY.to_owned())],
                index: 1,
                value_source: include_entry("fragments/shared.toml"),
            }],
        ),
    )
    .unwrap();
    assert_eq!(
        edited,
        "# kept\r\nincludes = [\r\n    \"fragments/core.toml\",\r\n    \"fragments/shared.toml\",\r\n]\r\nnotes = \"kept\"\r\nname = \"Test hull\"\r\n"
    );
}

// ── Refusals (criterion 4) ───────────────────────────────────────────────────

/// Every rule in the refusal set, each with the draft untouched. The message
/// opens with the rule so the panel maps it to a string and still shows what
/// was refused.
#[test]
fn every_refusal_leaves_the_source_untouched() {
    let files = draft();
    let refuse = |edits: Vec<Edit>| -> String {
        let before = files.clone();
        let error = compose(&files, &dependencies(), &request(&files, HULL, edits))
            .expect_err("the edit is refused");
        assert_eq!(files, before, "the draft is untouched");
        error
    };
    let add_include = |value: &str| {
        vec![Edit::Insert {
            path: vec![Segment::Key(INCLUDES_KEY.to_owned())],
            index: 2,
            value_source: include_entry(value),
        }]
    };

    let missing = refuse(add_include("fragments/gone.toml"));
    assert!(missing.starts_with("include-missing: "), "{missing}");
    // Relative to the declaring template, so the template's own file name.
    assert!(refuse(add_include("hull.toml")).starts_with("include-self: "));
    assert!(refuse(add_include("fragments/cycle.toml")).starts_with("include-cycle: "));
    assert!(refuse(add_include("../worlds/default.toml")).starts_with("include-disallowed: "));
    assert!(refuse(add_include("/absolute.toml")).starts_with("include-disallowed: "));

    // A component the runtime does not know — asked of the runtime, which
    // answers with serde's own sentence.
    let unsupported = refuse(vec![Edit::Put {
        path: vec![Segment::Key("warp_core".to_owned())],
        value_source: "{}".to_owned(),
    }]);
    assert!(
        unsupported.starts_with("component-unsupported: "),
        "{unsupported}"
    );
    assert!(
        unsupported.contains("unknown field `warp_core`"),
        "{unsupported}"
    );

    // A composed document that no longer parses, carrying the runtime's own
    // error rather than a sentence invented here.
    let invalid = refuse(vec![Edit::Put {
        path: vec![Segment::Key("tags".to_owned())],
        value_source: "5".to_owned(),
    }]);
    assert!(invalid.starts_with("entity-invalid: "), "{invalid}");

    // An inherited table has no tombstone the merge understands, so removing
    // it is refused with the owner named rather than silently leaving the
    // inherited copy in place.
    let inherited = refuse(vec![Edit::Remove {
        path: vec![Segment::Key("hull".to_owned())],
    }]);
    assert!(
        inherited.starts_with("component-inherited: "),
        "{inherited}"
    );
    assert!(inherited.contains(CORE), "{inherited}");

    let unknown = compose(
        &files,
        &dependencies(),
        &EntityEditRequest {
            document_path: SHARED.to_owned(),
            expected_source: dependencies().base_files[SHARED].clone(),
            edits: Vec::new(),
        },
    )
    .expect_err("a read-only dependency is not a draft member");
    assert!(unknown.starts_with("unknown-document: "), "{unknown}");
}

/// Adding and removing a component the closure does NOT inherit is the
/// ordinary local edit, and the skeleton it writes is the runtime's own.
#[test]
fn a_local_component_is_added_from_the_runtimes_skeleton_and_removed_again() {
    let mut files = draft();
    let skeleton = component_skeleton("reference_grid").unwrap();
    let added = compose(
        &files,
        &dependencies(),
        &request(
            &files,
            HULL,
            vec![Edit::Put {
                path: vec![Segment::Key("reference_grid".to_owned())],
                value_source: skeleton.to_owned(),
            }],
        ),
    )
    .unwrap();
    assert!(added.contains("reference_grid = {"), "{added}");
    assert!(added.starts_with("# The hull itself\n"), "{added}");

    files.insert(HULL.to_owned(), added);
    let removed = compose(
        &files,
        &dependencies(),
        &request(
            &files,
            HULL,
            vec![Edit::Remove {
                path: vec![Segment::Key("reference_grid".to_owned())],
            }],
        ),
    )
    .unwrap();
    assert_eq!(removed, hull());
}

/// A keyed array ENTRY does have a tombstone, so removing an inherited entry
/// is an ordinary local edit rather than the refusal a whole inherited table
/// earns.
#[test]
fn an_inherited_keyed_entry_is_removed_with_the_tombstone_the_merge_understands() {
    let files = draft();
    let edited = compose(
        &files,
        &dependencies(),
        &request(
            &files,
            HULL,
            vec![Edit::AppendTable {
                path: vec![Segment::Key("system".to_owned())],
                fields: vec![
                    ("id".to_owned(), "\"boost\"".to_owned()),
                    ("_remove".to_owned(), "true".to_owned()),
                ],
            }],
        ),
    )
    .unwrap();
    let mut after = files.clone();
    after.insert(HULL.to_owned(), edited);
    let mut sources = dependencies().base_files.clone();
    sources.extend(after.clone());
    let resolved = resolve_template(HULL, &sources).unwrap();
    // The entry is gone from the resolved document, and the marker with it.
    assert_eq!(resolved.provenance.origin("system[id=boost].kind"), None);
    assert!(!resolved.toml.contains("_remove"), "{}", resolved.toml);
}

// ── Materialising (criterion 2) ──────────────────────────────────────────────

/// Every byte of `before` survives in `after`, in order, with everything new
/// added in ONE run: `before` is `after` with one inserted stretch taken out.
fn assert_pure_insertion(before: &str, after: &str) {
    assert!(after.len() >= before.len(), "{after} is shorter");
    let prefix = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before[prefix..]
        .bytes()
        .rev()
        .zip(after[prefix..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    assert_eq!(
        prefix + suffix,
        before.len(),
        "the edit rewrote existing text rather than only adding: {before:?} -> {after:?}"
    );
}

/// Materialising writes NEW local text and nothing else: every original line
/// survives in order, and the value written is the resolved one.
#[test]
fn materialise_writes_only_new_local_text() {
    let files = draft();
    let edited = materialise(&files, &dependencies(), HULL, "hull.hull_integrity").unwrap();
    assert_eq!(
        edited,
        [
            "# The hull itself",
            "includes = [",
            "    \"fragments/core.toml\",",
            "    \"fragments/shared.toml\",",
            "]",
            "class = \"lancer\" # keep this tail",
            "name = \"Test hull\"",
            "hull = { hull_integrity = 120.0 }",
            "",
        ]
        .join("\n")
    );
    for line in hull().lines() {
        assert!(edited.contains(line), "{line} survives");
    }
    // …and it wrote nothing BUT new text: the original is a prefix and a suffix
    // of the result with one insertion between them, which is the property
    // contract D5 states and the one an edit that rewrote a span would break.
    assert_pure_insertion(&hull(), &edited);
    // The included file is never touched: the only source that comes back is
    // the local member's. (Structurally so — `materialise` hands back one
    // member's text — which is why the byte-identity of the OTHER members is
    // proved through the real draft in tests/smoke/workshop-entity.render.spec.js
    // rather than here.)
    assert_eq!(files[CORE], core());

    // …and the field is now this document's own, with its own line.
    let mut after = files.clone();
    after.insert(HULL.to_owned(), edited.clone());
    let catalog = catalog(&after, &dependencies(), HULL);
    let field = catalog
        .fields
        .iter()
        .find(|field| field.address == "hull.hull_integrity")
        .unwrap();
    assert_eq!(
        (field.local, field.source.as_str(), field.line),
        (true, HULL, Some(8))
    );
    assert_eq!(field.value_source, "120.0");

    // A second materialise of the same address is refused: it is local now.
    let error = materialise(&after, &dependencies(), HULL, "hull.hull_integrity")
        .expect_err("an already-local address is refused");
    assert!(error.starts_with("materialise-local: "), "{error}");
}

#[test]
fn materialise_serialises_an_inherited_array_whole() {
    let files = draft();
    let edited = materialise(&files, &dependencies(), HULL, "tags").unwrap();
    assert!(edited.contains("tags = [\"core\"]"), "{edited}");
}

/// A field inside a keyed array entry is materialised into the LOCAL entry the
/// merge reconciles by key — and refused when this template authors no such
/// entry, because the merge has nowhere to put it.
#[test]
fn materialise_into_a_keyed_array_needs_the_local_entry_first() {
    let files = draft();
    let error = materialise(&files, &dependencies(), HULL, "system[id=boost].kind")
        .expect_err("there is no local entry keyed id=boost");
    assert!(error.starts_with("materialise-keyed-entry: "), "{error}");
    assert!(error.contains("\"id\""), "{error}");

    let mut files = files;
    files.insert(
        HULL.to_owned(),
        format!("{}\n[[system]]\nid = \"boost\"\n", hull().trim_end()),
    );
    let edited = materialise(&files, &dependencies(), HULL, "system[id=boost].kind").unwrap();
    assert!(
        edited.ends_with("[[system]]\nid = \"boost\"\nkind = \"helm_boost\"\n"),
        "{edited}"
    );
}

/// Provenance falls back to a POSITION for an entry of a keyed array that
/// carries no key, and the merge APPENDS such an entry after the ones it
/// reconciled — so the resolved position and the local one are not the same
/// array at all. Refused wherever the position appears in the address, not only
/// as its last step: an indexed PARENT that happens to resolve locally would
/// write the value into whichever local entry sits at that position.
#[test]
fn materialise_refuses_a_position_in_an_array_the_merge_appends_to() {
    let mut files = draft();
    files.insert(
        CORE.to_owned(),
        format!("{}\n[[system]]\nkind = \"helm_boost\"\n", core().trim_end()),
    );
    files.insert(
        HULL.to_owned(),
        format!("{}\n[[system]]\nid = \"other\"\n", hull().trim_end()),
    );
    let catalog = catalog(&files, &dependencies(), HULL);
    let indexed = catalog
        .fields
        .iter()
        .find(|field| field.address.starts_with("system[") && !field.address.contains('='))
        .unwrap_or_else(|| {
            panic!(
                "a keyless entry is addressed by position: {:?}",
                catalog
                    .fields
                    .iter()
                    .map(|field| field.address.as_str())
                    .collect::<Vec<_>>()
            )
        });
    // The panel offers no Materialise for it, and the route refuses it.
    assert!(!indexed.materialisable, "{indexed:?}");
    let error = materialise(&files, &dependencies(), HULL, &indexed.address)
        .expect_err("a position is not an address");
    assert!(error.starts_with("materialise-keyed-entry: "), "{error}");
    // …and the local entry the position would have landed in is untouched.
    assert!(files[HULL].ends_with("[[system]]\nid = \"other\"\n"));
}

/// The catalog says which inherited fields Materialise can actually write, so
/// the panel offers the control exactly where the runtime answers: a field
/// inside a keyed array entry this template does not author is refused, and the
/// row says why instead of holding a button whose every press fails.
#[test]
fn a_field_the_local_document_cannot_name_yet_is_not_materialisable() {
    let files = draft();
    let catalog = catalog(&files, &dependencies(), HULL);
    let field = |address: &str| {
        catalog
            .fields
            .iter()
            .find(|field| field.address == address)
            .unwrap_or_else(|| panic!("{address} is a field"))
    };
    // No local `[[system]]` entry keyed id=boost: there is nowhere to put it.
    let keyed = field("system[id=boost].kind");
    assert_eq!((keyed.local, keyed.materialisable), (false, false));
    assert!(materialise(&files, &dependencies(), HULL, &keyed.address)
        .expect_err("refused")
        .starts_with("materialise-keyed-entry: "));
    // An ordinary inherited field one absent table deep IS materialisable, and
    // a local one never is — it is already authored here.
    assert!(field("hull.hull_integrity").materialisable);
    assert!(!field("class").materialisable);
    assert!(materialise(&files, &dependencies(), HULL, "hull.hull_integrity").is_ok());
}

#[test]
fn materialise_refuses_an_address_the_resolved_document_does_not_carry() {
    let files = draft();
    for address in ["hull.nonesuch", "", "nonesuch"] {
        let error = materialise(&files, &dependencies(), HULL, address)
            .expect_err("an unknown address is refused");
        assert!(
            error.starts_with("materialise-unknown-address: "),
            "{address}: {error}"
        );
    }
    let error = materialise(
        &files,
        &dependencies(),
        SHARED,
        "repair.repair_rate_hp_per_sec",
    )
    .expect_err("a read-only dependency is not a draft member");
    assert!(error.starts_with("unknown-document: "), "{error}");
}

// ── Findings (section D) ─────────────────────────────────────────────────────

#[test]
fn findings_locate_every_include_rule_at_its_entry_line() {
    let mut files = draft();
    files.insert(
        HULL.to_owned(),
        [
            "includes = [",
            "    \"fragments/gone.toml\",",
            "    \"hull.toml\",",
            "    \"../worlds/default.toml\",",
            "    \"/absolute.toml\",",
            "    \"fragments/cycle.toml\",",
            "]",
            "name = \"Test hull\"",
            "",
        ]
        .join("\n"),
    );
    let findings = findings(&files, &dependencies().base_files);
    assert_eq!(
        categories(&findings, HULL),
        vec![
            ("include-missing".to_owned(), Some(2)),
            ("include-self".to_owned(), Some(3)),
            ("include-disallowed".to_owned(), Some(4)),
            ("include-disallowed".to_owned(), Some(5)),
            ("include-cycle".to_owned(), Some(6)),
        ]
    );
    assert!(findings.iter().all(|finding| finding.severity == "error"));
    // The other half of the cycle is reported on the file that closes it.
    assert_eq!(
        categories(&findings, CYCLE),
        vec![("include-cycle".to_owned(), Some(1))]
    );
    // Deterministic: the same inputs, the same order, no duplicates.
    assert_eq!(findings, self::findings(&files, &dependencies().base_files));
}

/// A composed template whose resolved document is not an entity is reported
/// once, with the runtime's own sentence and the include chain. The fragment
/// that broke it is NOT, because it composed nothing itself: a partial
/// fragment is legitimate, and the source gate owns its own errors.
#[test]
fn a_composed_template_that_is_not_an_entity_is_unresolvable_and_a_fragment_is_not() {
    let mut files = draft();
    files.remove(CYCLE);
    files.insert(
        CORE.to_owned(),
        core().replace("tags = [\"core\"]", "tags = 5"),
    );
    let findings = findings(&files, &dependencies().base_files);
    assert_eq!(
        categories(&findings, HULL),
        vec![("entity-unresolvable".to_owned(), Some(2))]
    );
    let hull_finding = findings
        .iter()
        .find(|finding| finding.file == HULL)
        .unwrap();
    assert!(
        hull_finding.message.contains("include chain"),
        "{}",
        hull_finding.message
    );
    // The fragment itself is not condemned for not being a complete entity.
    assert!(categories(&findings, CORE).is_empty(), "{findings:?}");
}

#[test]
fn an_uncomposed_partial_template_is_not_a_finding() {
    let mut files = draft();
    // A lone fragment nothing includes: incomplete on its own, and reported by
    // the source gate rather than as a composition failure.
    files.insert(
        "assets/entities/fragments/lonely.toml".to_owned(),
        "[collider]\nradius = 3.0\n".to_owned(),
    );
    assert!(findings(&files, &dependencies().base_files).is_empty());
}

/// The shipped project exactly as `validate_project` sees it yields no entity
/// composition finding, so a Check of the untouched project is quiet.
#[test]
fn shipped_content_has_no_entity_findings() {
    let mut candidate = BTreeMap::new();
    text_members_under("assets", &mut candidate);
    for expected in [
        "assets/entities/alliance_cruiser.toml",
        "assets/entities/fragments/ai/fleet_baseline.toml",
    ] {
        assert!(candidate.contains_key(expected), "{expected} is shipped");
    }
    let findings = findings(&candidate, &BTreeMap::new());
    assert!(findings.is_empty(), "{findings:?}");
    // …and a shipped composed hull reads back through the catalog with its
    // fragments in merge order and an inherited field the panel can show.
    let catalog = catalog(
        &candidate,
        &WorkshopDependencies::default(),
        "assets/entities/alliance_cruiser.toml",
    );
    assert!(catalog.resolvable, "{:?}", catalog.error);
    assert!(catalog.sources.len() > 1, "{:?}", catalog.sources);
    assert!(catalog
        .fields
        .iter()
        .any(|field| !field.local && field.line.is_none()));
    assert!(catalog.fields.iter().any(|field| field.local));
}

/// The same two readings over SHIPPED content, because this is not a fixture
/// shape: every hull that overrides part of an inherited component is local and
/// inherited at once, and the control the panel renders for it has to be one the
/// runtime accepts.
#[test]
fn a_shipped_composed_hull_is_local_and_inherited_and_its_override_can_be_dropped() {
    const CRUISER: &str = "assets/entities/alliance_cruiser.toml";
    let mut candidate = BTreeMap::new();
    text_members_under("assets", &mut candidate);
    let dependencies = WorkshopDependencies::default();
    let catalog = catalog(&candidate, &dependencies, CRUISER);
    assert!(catalog.resolvable, "{:?}", catalog.error);
    let pairs: Vec<&str> = catalog
        .components
        .iter()
        .filter(|component| component.local && component.inherited_from.is_some())
        .map(|component| component.key.as_str())
        .collect();
    assert!(
        pairs.contains(&"helm_console") && pairs.len() > 1,
        "{pairs:?}"
    );
    // The hull authors `[helm_console]`; the fragments it composes author
    // `helm_console.engines_ai.*`. Neither reading may turn the row's Remove
    // into a control whose every press is refused.
    compose(
        &candidate,
        &dependencies,
        &request(&candidate, CRUISER, remove_component("helm_console")),
    )
    .expect("dropping the hull's own helm_console text is an ordinary edit");
}

fn text_members_under(directory: &str, candidate: &mut BTreeMap<String, String>) {
    for entry in std::fs::read_dir(directory).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            text_members_under(&format!("{directory}/{name}"), candidate);
        } else if name.ends_with(".toml") || name.ends_with(".rhai") {
            candidate.insert(
                format!("{directory}/{name}"),
                std::fs::read_to_string(&path).unwrap(),
            );
        }
    }
}

// ── Address parsing ──────────────────────────────────────────────────────────

#[test]
fn provenance_addresses_parse_back_into_steps() {
    assert_eq!(
        parse_address("station[id=bridge].rating[name=Std].automated_systems"),
        Some(vec![
            Step::Keyed {
                name: "station".to_owned(),
                by: "id".to_owned(),
                value: "bridge".to_owned()
            },
            Step::Keyed {
                name: "rating".to_owned(),
                by: "name".to_owned(),
                value: "Std".to_owned()
            },
            Step::Key("automated_systems".to_owned()),
        ])
    );
    // Provenance quotes a key that carries a dot, a bracket or a space.
    assert_eq!(
        parse_address("\"odd.key\".child"),
        Some(vec![
            Step::Key("odd.key".to_owned()),
            Step::Key("child".to_owned()),
        ])
    );
    assert_eq!(
        parse_address("system[2]"),
        Some(vec![Step::Indexed {
            name: "system".to_owned(),
            index: 2
        }])
    );
    // The identity key comes from the merge's own table, not from a copy here.
    assert_eq!(
        identity_of(&[
            Step::Keyed {
                name: "station".to_owned(),
                by: "id".to_owned(),
                value: "bridge".to_owned()
            },
            Step::Keyed {
                name: "rating".to_owned(),
                by: "name".to_owned(),
                value: "Std".to_owned()
            },
        ]),
        "name"
    );
    for malformed in ["", ".", "a..b", "a[", "a]", "\"unclosed"] {
        assert_eq!(parse_address(malformed), None, "{malformed}");
    }
}
