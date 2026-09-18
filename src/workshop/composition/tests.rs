use super::*;
use crate::workshop::document::{Edit, Segment};
use crate::workshop::WorkshopDependencyPack;

const MANIFEST_PATH: &str = "scenarios.toml";
const ROOT: &str = "assets/worlds/root.toml";
const CHILD: &str = "assets/worlds/child.toml";
const BASE_CHILD: &str = "assets/worlds/base_child.toml";
const BASE_CHILD_SCRIPT: &str = "assets/worlds/base_child.rhai";
const BASE_ROOT: &str = "assets/worlds/base_root.toml";
const PACK_WORLD: &str = "assets/worlds/pack_world.toml";
const HULL: &str = "assets/entities/hull.toml";
const GHOST: &str = "assets/entities/ghost.toml";

/// A complete hull with a ship configuration: the one kind of template a
/// Test may select and a world may offer.
const HULL_SOURCE: &str = "class='lancer'\nname='Test hull'\n\
[[station]]\nid='captain'\nname='Captain'\ndescription='Test station'\nrank='captain'\n\
[[system]]\nid='boost'\nkind='helm_boost'\nstation='captain'\n";

/// A pack manifest with an unknown top-level key, two roots (one in the
/// draft, one beneath it) and a curated ship the world does not offer.
fn manifest() -> String {
    [
        "notes = \"kept\"",
        "",
        "[pack]",
        "format = 1",
        "id = \"aurora\"",
        "name = \"Aurora\"",
        "version = \"1.0.0\"",
        "",
        "[pack.requires]",
        "content_id = \"phoenix-base\"",
        "content_epoch = 1",
        "",
        "[[scenario]]",
        "id = \"root\"",
        "world = \"assets/worlds/root.toml\"",
        "label = \"Root\"",
        "ships = [",
        "    \"assets/entities/hull.toml\",",
        "    \"assets/entities/ghost.toml\",",
        "]",
        "",
        "[[scenario]]",
        "id = \"based\"",
        "world = \"assets/worlds/base_root.toml\"",
        "",
    ]
    .join("\n")
}

/// The draft root: two children (one beneath), an offered hull, and an
/// inline script naming a missing layer and a base child.
fn root() -> String {
    [
        "extra_worlds = [",
        "    \"assets/worlds/child.toml\",",
        "    \"assets/worlds/base_child.toml\",",
        "]",
        "",
        "[global]",
        "title = \"Root\"",
        "",
        "[[available_ships]]",
        "template_path = \"assets/entities/hull.toml\"",
        "",
        "[script]",
        "setup = '''",
        "// ctx.effects.load_world(\"assets/worlds/commented.toml\");",
        "fn go(ctx) {",
        "    ctx.effects.load_world(\"assets/worlds/layer.toml\");",
        "    ctx.delay.unload_world(\"assets/worlds/base_child.toml\");",
        "}",
        "'''",
        "",
    ]
    .join("\n")
}

/// The draft child carries a TOML action, which the runtime ignores at the
/// top level but the scan lists.
fn child() -> String {
    "[global]\ntitle = \"Child\"\n\n[[action]]\ntype = \"load_world\"\npath = \"assets/worlds/root.toml\"\n".into()
}

fn dependencies() -> WorkshopDependencies {
    WorkshopDependencies {
        base_files: BTreeMap::from([
            (
                BASE_CHILD.into(),
                "script = \"base_child.rhai\"\n[global]\ntitle = \"Base child\"\n".into(),
            ),
            (
                BASE_CHILD_SCRIPT.into(),
                "fn arm(ctx) {\n    ctx.effects.load_world(\"assets/worlds/root.toml\");\n    ctx.effects.unload_world(\"assets/worlds/gone.toml\");\n}\n".into(),
            ),
            (BASE_ROOT.into(), "[global]\ntitle = \"Base root\"\n".into()),
        ]),
        base_assets: BTreeMap::new(),
        packs: vec![WorkshopDependencyPack {
            id: "extra".into(),
            manifest_toml: String::new(),
            files: BTreeMap::from([(PACK_WORLD.into(), "[global]\ntitle = \"Packed\"\n".into())]),
            assets: BTreeMap::new(),
        }],
    }
}

fn files(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect()
}

fn draft() -> BTreeMap<String, String> {
    files(&[
        (MANIFEST_PATH, &manifest()),
        (ROOT, &root()),
        (CHILD, &child()),
        (HULL, HULL_SOURCE),
    ])
}

fn located(findings: &[WorkshopFinding], category: &str) -> Vec<(String, Option<usize>)> {
    findings
        .iter()
        .filter(|finding| finding.category == category)
        .map(|finding| (finding.file.clone(), finding.line))
        .collect()
}

fn key(text: &str) -> Segment {
    Segment::Key(text.into())
}

fn request(document_path: &str, expected_source: &str, edits: Vec<Edit>) -> ComposeRequest {
    ComposeRequest {
        document_path: document_path.into(),
        expected_source: expected_source.into(),
        edits,
    }
}

#[test]
fn catalog_carries_the_manifest_with_lines_headers_and_offered_ships() {
    let catalog = catalog(&draft(), &dependencies());
    let manifest = catalog.manifest.as_ref().unwrap();
    assert_eq!(manifest.path, MANIFEST_PATH);
    assert_eq!(manifest.kind, "pack");
    let pack = manifest.pack.as_ref().unwrap();
    assert_eq!(
        (
            pack.id.as_str(),
            pack.name.as_str(),
            pack.version.as_str(),
            pack.line
        ),
        ("aurora", "Aurora", "1.0.0", 3)
    );
    assert!(manifest.content.is_none());
    assert_eq!(manifest.unknown_keys, vec!["notes".to_owned()]);
    assert_eq!(manifest.scenarios.len(), 2);
    let first = &manifest.scenarios[0];
    assert_eq!(
        (first.index, first.id.as_str(), first.id_line),
        (0, "root", 14)
    );
    assert_eq!((first.world.as_str(), first.world_line), (ROOT, 15));
    assert_eq!(first.world_origin.as_deref(), Some("draft"));
    assert_eq!(first.label.as_deref(), Some("Root"));
    assert_eq!(
        first
            .ships
            .iter()
            .map(|ship| (ship.path.as_str(), ship.line, ship.offered))
            .collect::<Vec<_>>(),
        vec![(HULL, 18, true), (GHOST, 19, false)]
    );
    assert_eq!(first.offered_ships, vec![HULL.to_owned()]);
    let second = &manifest.scenarios[1];
    assert_eq!((second.id.as_str(), second.id_line), ("based", 23));
    assert_eq!((second.world.as_str(), second.world_line), (BASE_ROOT, 24));
    assert_eq!(second.world_origin.as_deref(), Some("base"));
    assert!(second.label.is_none());
    assert!(second.ships.is_empty() && second.offered_ships.is_empty());
}

#[test]
fn catalog_lists_worlds_draft_first_with_children_script_refs_and_origins() {
    let catalog = catalog(&draft(), &dependencies());
    assert_eq!(
        catalog
            .worlds
            .iter()
            .map(|world| (world.path.as_str(), world.origin.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (CHILD, "draft"),
            (ROOT, "draft"),
            (BASE_CHILD, "base"),
            (BASE_ROOT, "base"),
            (PACK_WORLD, "pack:extra"),
        ]
    );
    let root = &catalog.worlds[1];
    assert_eq!((root.title.as_deref(), root.line), (Some("Root"), 1));
    assert_eq!(
        root.extra_worlds
            .iter()
            .map(|extra| (
                extra.index,
                extra.path.as_str(),
                extra.line,
                extra.origin.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            (0, CHILD, 2, Some("draft")),
            (1, BASE_CHILD, 3, Some("base"))
        ]
    );
    let refs = |world: &WorldView| {
        world
            .script_refs
            .iter()
            .map(|reference| {
                (
                    reference.path.clone(),
                    reference.line,
                    reference.kind.clone(),
                    reference.source.clone(),
                    reference.origin.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    // The commented call is skipped; the delayed variant is a reference too.
    assert_eq!(
        refs(root),
        vec![
            (
                "assets/worlds/layer.toml".into(),
                16,
                "load".into(),
                "inline-script".into(),
                None
            ),
            (
                BASE_CHILD.into(),
                17,
                "unload".into(),
                "inline-script".into(),
                Some("base".into())
            ),
        ]
    );
    assert_eq!(
        root.available_ships
            .iter()
            .map(|ship| (
                ship.template_path.as_str(),
                ship.line,
                ship.origin.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![(HULL, 10, Some("draft"))]
    );
    assert_eq!(
        refs(&catalog.worlds[0]),
        vec![(
            ROOT.into(),
            6,
            "load".into(),
            "trigger".into(),
            Some("draft".into())
        )]
    );
    // A sibling script's references carry the sibling's path and lines.
    assert_eq!(
        refs(&catalog.worlds[2]),
        vec![
            (
                ROOT.into(),
                2,
                "load".into(),
                BASE_CHILD_SCRIPT.into(),
                Some("draft".into())
            ),
            (
                "assets/worlds/gone.toml".into(),
                3,
                "unload".into(),
                BASE_CHILD_SCRIPT.into(),
                None
            ),
        ]
    );
    assert!(catalog.worlds[3].script_refs.is_empty());
}

#[test]
fn catalog_lists_members_choices_catalogue_and_findings() {
    let catalog = catalog(&draft(), &dependencies());
    assert_eq!(
        catalog
            .members
            .iter()
            .map(|member| (
                member.path.as_str(),
                member.origin.as_str(),
                member.allowed,
                member.referenced_by.clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                HULL,
                "draft",
                true,
                vec![ROOT.to_owned(), MANIFEST_PATH.to_owned()]
            ),
            (CHILD, "draft", true, vec![ROOT.to_owned()]),
            (
                ROOT,
                "draft",
                true,
                vec![
                    BASE_CHILD.to_owned(),
                    CHILD.to_owned(),
                    MANIFEST_PATH.to_owned()
                ]
            ),
            (MANIFEST_PATH, "draft", true, vec![]),
            (BASE_CHILD_SCRIPT, "base", true, vec![]),
            (BASE_CHILD, "base", true, vec![ROOT.to_owned()]),
            (BASE_ROOT, "base", true, vec![MANIFEST_PATH.to_owned()]),
            (PACK_WORLD, "pack:extra", true, vec![]),
        ]
    );
    assert_eq!(
        catalog
            .choices
            .worlds
            .iter()
            .map(|choice| (choice.path.as_str(), choice.origin.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (CHILD, "draft"),
            (ROOT, "draft"),
            (BASE_CHILD, "base"),
            (BASE_ROOT, "base"),
            (PACK_WORLD, "pack:extra"),
        ]
    );
    assert_eq!(
        catalog
            .choices
            .templates
            .iter()
            .map(|choice| (choice.path.as_str(), choice.origin.as_str()))
            .collect::<Vec<_>>(),
        vec![(HULL, "draft")]
    );
    assert_eq!(
        catalog.catalogue,
        vec![
            CatalogueEntry {
                id: "root".into(),
                world: ROOT.into(),
                label: Some("Root".into()),
                description: None,
                ships: vec![CatalogueShip {
                    template_path: HULL.into(),
                    label: None
                }],
                origin: None,
            },
            CatalogueEntry {
                id: "based".into(),
                world: BASE_ROOT.into(),
                label: Some("Base root".into()),
                description: None,
                ships: Vec::new(),
                origin: None,
            },
        ]
    );
    // The missing layer is the one finding; a read-only script's dangling
    // reference is not the draft's to fix and is listed, not reported.
    assert_eq!(
        catalog
            .findings
            .iter()
            .map(|finding| (
                finding.category.as_str(),
                finding.file.as_str(),
                finding.line
            ))
            .collect::<Vec<_>>(),
        vec![("world-missing-load-reference", ROOT, Some(16))]
    );
}

#[test]
fn a_project_manifest_is_read_with_its_content_header_and_an_unreadable_draft_is_reported() {
    let catalog = catalog(
        &files(&[
            (
                "assets/scenarios.toml",
                "[content]\nid = \"base\"\nepoch = 3\n\n[[scenario]]\nid = \"one\"\nworld = \"assets/worlds/one.toml\"\n",
            ),
            ("assets/worlds/one.toml", "[global\ntitle = \"broken\"\n"),
        ]),
        &WorkshopDependencies::default(),
    );
    let manifest = catalog.manifest.as_ref().unwrap();
    assert_eq!(manifest.path, "assets/scenarios.toml");
    assert_eq!(manifest.kind, "project");
    let content = manifest.content.as_ref().unwrap();
    assert_eq!(
        (content.id.as_str(), content.epoch, content.line),
        ("base", Some(3), 1)
    );
    assert!(manifest.pack.is_none());
    assert_eq!(manifest.scenarios[0].world_origin.as_deref(), Some("draft"));
    assert_eq!(
        located(&catalog.findings, "runtime-source-invalid"),
        vec![("assets/worlds/one.toml".to_owned(), Some(1))]
    );
    assert!(
        catalog.catalogue.is_empty(),
        "an unparsable world is not catalogued"
    );
}

/// A draft without a manifest has none — not a phantom `scenarios.toml` the
/// panel would offer roots on and then be refused for — and a nested world
/// (a project workspace admits any assets/**.toml) is listed but never
/// offered as a root or child, since the rules would refuse it on Add.
#[test]
fn a_draft_without_a_manifest_has_none_and_a_nested_world_is_not_a_choice() {
    let catalog = catalog(
        &files(&[
            ("assets/worlds/sub/nested.toml", "[global]\n"),
            (ROOT, "[global]\n"),
        ]),
        &WorkshopDependencies::default(),
    );
    assert!(catalog.manifest.is_none());
    assert!(catalog.catalogue.is_empty());
    assert_eq!(
        catalog
            .worlds
            .iter()
            .map(|world| world.path.as_str())
            .collect::<Vec<_>>(),
        vec![ROOT, "assets/worlds/sub/nested.toml"]
    );
    assert_eq!(
        catalog
            .choices
            .worlds
            .iter()
            .map(|choice| choice.path.as_str())
            .collect::<Vec<_>>(),
        vec![ROOT]
    );
}

#[test]
fn compose_refuses_each_manifest_rule_and_accepts_a_valid_edit() {
    let clean = manifest().replace("    \"assets/entities/ghost.toml\",\n", "");
    let mut draft = draft();
    draft.insert(MANIFEST_PATH.into(), clean.clone());
    let dependencies = dependencies();
    let append = |fields: &[(&str, &str)]| {
        request(
            MANIFEST_PATH,
            &clean,
            vec![Edit::AppendTable {
                path: vec![key("scenario")],
                fields: fields
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
            }],
        )
    };
    for (rule, edits) in [
        (
            "duplicate-scenario-id",
            append(&[
                ("id", "\"root\""),
                ("world", "\"assets/worlds/child.toml\""),
            ]),
        ),
        (
            "invalid-manifest-entry",
            append(&[("id", "\"\""), ("world", "\"assets/worlds/child.toml\"")]),
        ),
        (
            "invalid-manifest-entry",
            append(&[("id", "\"new\""), ("world", "\"\"")]),
        ),
        (
            "scenario-world-disallowed",
            append(&[
                ("id", "\"new\""),
                ("world", "\"assets/entities/hull.toml\""),
            ]),
        ),
        (
            "missing-scenario-world",
            append(&[("id", "\"new\""), ("world", "\"assets/worlds/nope.toml\"")]),
        ),
        (
            "unknown-scenario-ship",
            request(
                MANIFEST_PATH,
                &clean,
                vec![Edit::Insert {
                    path: vec![key("scenario"), Segment::Index(0), key("ships")],
                    index: 1,
                    value_source: "\"assets/entities/ghost.toml\"".into(),
                }],
            ),
        ),
        (
            "manifest-header-removed",
            request(
                MANIFEST_PATH,
                &clean,
                vec![Edit::Remove {
                    path: vec![key("pack")],
                }],
            ),
        ),
    ] {
        let refused = compose(&draft, &dependencies, &edits).unwrap_err();
        assert!(refused.starts_with(rule), "{rule}: {refused}");
    }
    let accepted = compose(
        &draft,
        &dependencies,
        &request(
            MANIFEST_PATH,
            &clean,
            vec![Edit::Set {
                path: vec![key("scenario"), Segment::Index(0), key("label")],
                value_source: "\"Renamed\"".into(),
            }],
        ),
    )
    .unwrap();
    assert!(accepted.contains("label = \"Renamed\""), "{accepted}");
    assert_eq!(
        draft[MANIFEST_PATH], clean,
        "the draft's source is untouched"
    );
    // A rule the manifest already broke is a finding, not a bar to every
    // other edit: the ghost ship stays and the label still changes.
    let mut dirty_manifest = draft.clone();
    dirty_manifest.insert(MANIFEST_PATH.into(), manifest());
    let relabelled = compose(
        &dirty_manifest,
        &dependencies,
        &request(
            MANIFEST_PATH,
            &manifest(),
            vec![Edit::Set {
                path: vec![key("scenario"), Segment::Index(0), key("label")],
                value_source: "\"Renamed\"".into(),
            }],
        ),
    )
    .unwrap();
    assert!(relabelled.contains("ghost.toml") && relabelled.contains("Renamed"));
}

/// Every refusal message names the entry's array slot, so an edit that only
/// moves a hand-broken entry to another slot must be judged by what it
/// introduced, not by the message it re-indexed: removing the entry before
/// a missing child, removing the root before a root whose world is missing,
/// and the panel's move-as-`set`s past a root with an unoffered ship all
/// introduce nothing. A SECOND copy of a violation the member already had is
/// new and is refused — the manifest half proves that on its own count
/// (two roots naming one missing world), and the `extra_worlds` half shows
/// which rule a repeat is charged with when it both duplicates and re-misses.
#[test]
fn compose_does_not_charge_an_edit_with_a_violation_it_only_shifted() {
    let root =
        "extra_worlds = [\"assets/worlds/child.toml\", \"assets/worlds/nope.toml\"]\n[global]\n";
    let mut draft = draft();
    draft.insert(ROOT.into(), root.into());
    let dependencies = dependencies();
    let removed = compose(
        &draft,
        &dependencies,
        &request(
            ROOT,
            root,
            vec![Edit::Remove {
                path: vec![key("extra_worlds"), Segment::Index(0)],
            }],
        ),
    )
    .unwrap();
    assert!(
        removed.starts_with("extra_worlds = [\"assets/worlds/nope.toml\"]"),
        "{removed}"
    );
    let doubled = compose(
        &draft,
        &dependencies,
        &request(
            ROOT,
            root,
            vec![Edit::Insert {
                path: vec![key("extra_worlds")],
                index: 0,
                value_source: "\"assets/worlds/nope.toml\"".into(),
            }],
        ),
    )
    .unwrap_err();
    // Refused for the DUPLICATE, not for the missing world. The list already
    // carried one `extra-worlds-missing` for `nope.toml`, and the before/after
    // comparison is a multiset of (category, key): the missing-ness is absorbed
    // by the count the member already had, whichever slot now carries it, and
    // the only violation left with no counterpart is the second copy. Naming
    // the missing world here would charge the author for a problem they did not
    // just create.
    assert!(doubled.starts_with("extra-worlds-duplicate"), "{doubled}");

    // Roots: the second root's world is missing and its ship is not offered
    // by anything; removing the first root and swapping the two are both
    // accepted, while giving the FIRST root the same missing world is not.
    let manifest = [
        "[pack]",
        "format = 1",
        "id = \"aurora\"",
        "",
        "[[scenario]]",
        "id = \"root\"",
        "world = \"assets/worlds/root.toml\"",
        "",
        "[[scenario]]",
        "id = \"broken\"",
        "world = \"assets/worlds/nope.toml\"",
        "ships = [\"assets/entities/ghost.toml\"]",
        "",
    ]
    .join("\n");
    draft.insert(MANIFEST_PATH.into(), manifest.clone());
    let removed = compose(
        &draft,
        &dependencies,
        &request(
            MANIFEST_PATH,
            &manifest,
            vec![Edit::Remove {
                path: vec![key("scenario"), Segment::Index(0)],
            }],
        ),
    )
    .unwrap();
    assert!(
        !removed.contains("id = \"root\"") && removed.contains("id = \"broken\""),
        "{removed}"
    );
    let set = |slot: usize, field: &str, value: &str| Edit::Set {
        path: vec![key("scenario"), Segment::Index(slot), key(field)],
        value_source: format!("\"{value}\""),
    };
    let swapped = compose(
        &draft,
        &dependencies,
        &request(
            MANIFEST_PATH,
            &manifest,
            vec![
                set(0, "id", "broken"),
                set(0, "world", "assets/worlds/nope.toml"),
                Edit::Put {
                    path: vec![key("scenario"), Segment::Index(0), key("ships")],
                    value_source: "[\"assets/entities/ghost.toml\"]".into(),
                },
                set(1, "id", "root"),
                set(1, "world", "assets/worlds/root.toml"),
                Edit::Remove {
                    path: vec![key("scenario"), Segment::Index(1), key("ships")],
                },
            ],
        ),
    )
    .unwrap();
    assert!(
        swapped.contains("id = \"broken\"\nworld = \"assets/worlds/nope.toml\""),
        "{swapped}"
    );
    let spread = compose(
        &draft,
        &dependencies,
        &request(
            MANIFEST_PATH,
            &manifest,
            vec![set(0, "world", "assets/worlds/nope.toml")],
        ),
    )
    .unwrap_err();
    assert!(spread.starts_with("missing-scenario-world"), "{spread}");
    assert_eq!(
        draft[MANIFEST_PATH], manifest,
        "the draft's source is untouched"
    );
}

#[test]
fn compose_refuses_each_extra_worlds_rule_over_candidate_and_dependencies() {
    let root = "extra_worlds = [\"assets/worlds/child.toml\"]\n\n[global]\ntitle = \"Root\"\n";
    let mut draft = draft();
    draft.insert(ROOT.into(), root.into());
    // The candidate's loop world points back at the root; the base copy of
    // the same path does not, so the cycle exists only because the
    // candidate wins the path.
    draft.insert(
        "assets/worlds/loop.toml".into(),
        "extra_worlds = [\"assets/worlds/root.toml\"]\n[global]\n".into(),
    );
    let mut dependencies = dependencies();
    dependencies
        .base_files
        .insert("assets/worlds/loop.toml".into(), "[global]\n".into());
    let insert = |value: &str| {
        request(
            ROOT,
            root,
            vec![Edit::Insert {
                path: vec![key("extra_worlds")],
                index: 1,
                value_source: format!("\"{value}\""),
            }],
        )
    };
    for (rule, value) in [
        ("extra-worlds-missing", "assets/worlds/nope.toml"),
        ("extra-worlds-duplicate", CHILD),
        ("extra-worlds-self", ROOT),
        ("extra-worlds-disallowed", HULL),
        ("extra-worlds-cycle", "assets/worlds/loop.toml"),
    ] {
        let refused = compose(&draft, &dependencies, &insert(value)).unwrap_err();
        assert!(refused.starts_with(rule), "{rule}: {refused}");
        if rule == "extra-worlds-cycle" {
            assert!(
                refused.contains(
                    "assets/worlds/root.toml -> assets/worlds/loop.toml -> assets/worlds/root.toml"
                ),
                "{refused}"
            );
        }
    }
    assert_eq!(draft[ROOT], root, "the draft's source is untouched");
    let accepted = compose(&draft, &dependencies, &insert(BASE_CHILD)).unwrap();
    assert!(
        accepted.starts_with(
            "extra_worlds = [\"assets/worlds/child.toml\", \"assets/worlds/base_child.toml\"]\n"
        ),
        "{accepted}"
    );
    let stale = compose(&draft, &dependencies, &request(ROOT, "[global]\n", vec![])).unwrap_err();
    assert!(stale.contains("changed"), "{stale}");
    let unknown = compose(
        &draft,
        &dependencies,
        &request("assets/worlds/elsewhere.toml", "", vec![]),
    )
    .unwrap_err();
    assert!(unknown.starts_with("unknown-document"), "{unknown}");
}

#[test]
fn every_finding_category_reports_its_exact_line() {
    let candidate = files(&[
        (
            "assets/scenarios.toml",
            "[content]\nid = \"base\"\nepoch = 1\n\n[[scenario]]\nid = \"bad\"\nworld = \"assets/entities/hull.toml\"\n\n[[scenario]]\nid = \"ok\"\nworld = \"assets/worlds/a.toml\"\n",
        ),
        (
            "assets/worlds/a.toml",
            "extra_worlds = [\n    \"assets/worlds/b.toml\",\n    \"assets/worlds/a.toml\",\n    \"assets/worlds/b.toml\",\n    \"assets/entities/hull.toml\",\n    \"assets/worlds/missing.toml\",\n]\n[global]\n[[available_ships]]\ntemplate_path = \"assets/entities/nested/hull.toml\"\n",
        ),
        (
            "assets/worlds/b.toml",
            "extra_worlds = [\"assets/worlds/c.toml\"]\n[global]\n",
        ),
        // Comments of every shape hide a call — a leading `//`, a block
        // (nested, spanning lines), a trailing one after code — and a `//`
        // inside a string does not open one.
        (
            "assets/worlds/d.rhai",
            "// ctx.effects.load_world(\"assets/worlds/nope.toml\");\n/* ctx.effects.load_world(\"assets/worlds/nope.toml\");\n   /* nested */ still a comment */\nfn f(ctx) { ctx.log(\"http://x\"); ctx.effects.load_world(\"assets/worlds/nope.toml\"); } // load_world(\"assets/worlds/trailing.toml\")\nfn g(ctx) { ctx.delay.unload_world(\"assets/worlds/a.toml\"); }\n",
        ),
        // A call in prose is not a composition; a trigger action nested
        // under an entity is, at its path's line.
        (
            "assets/worlds/e.toml",
            "[global]\ndescription = \"see load_world(\\\"assets/worlds/prose.toml\\\")\"\n[[entity]]\nname = \"x\"\non_destroyed = [{ type = \"unload_world\", path = \"assets/worlds/nope.toml\" }]\n",
        ),
        // A one-line basic-string body escapes its quotes; the runtime
        // still resolves the call, so it is listed at the body's line.
        (
            "assets/worlds/f.toml",
            "[global]\n[script]\nsetup = \"fn go(ctx) { ctx.effects.load_world(\\\"assets/worlds/nope.toml\\\"); }\"\n",
        ),
        // Only a member the composition NAMES is warned about: the nested
        // hull a.toml offers, never the unreferenced join codes.
        ("assets/entities/nested/hull.toml", HULL_SOURCE),
        ("assets/join/codes.toml", "value = 1\n"),
        (HULL, HULL_SOURCE),
    ]);
    // The cycle closes through a read-only world, which carries no finding
    // of its own: nothing the author can edit there.
    let beneath = files(&[(
        "assets/worlds/c.toml",
        "extra_worlds = [\"assets/worlds/a.toml\"]\n",
    )]);
    let findings = findings(&candidate, &beneath);
    let a = "assets/worlds/a.toml".to_owned();
    assert_eq!(
        located(&findings, "extra-worlds-cycle"),
        vec![
            (a.clone(), Some(2)),
            ("assets/worlds/b.toml".to_owned(), Some(1))
        ]
    );
    let cycle = findings
        .iter()
        .find(|finding| finding.category == "extra-worlds-cycle" && finding.file == a)
        .unwrap();
    assert!(
        cycle.message.contains(
            "assets/worlds/a.toml -> assets/worlds/b.toml -> assets/worlds/c.toml -> assets/worlds/a.toml"
        ),
        "{}",
        cycle.message
    );
    assert_eq!(
        located(&findings, "extra-worlds-self"),
        vec![(a.clone(), Some(3))]
    );
    assert_eq!(
        located(&findings, "extra-worlds-duplicate"),
        vec![(a.clone(), Some(4))]
    );
    assert_eq!(
        located(&findings, "extra-worlds-disallowed"),
        vec![(a.clone(), Some(5))]
    );
    assert_eq!(
        located(&findings, "extra-worlds-missing"),
        vec![(a, Some(6))]
    );
    assert_eq!(
        located(&findings, "world-missing-load-reference"),
        vec![
            ("assets/worlds/d.rhai".to_owned(), Some(4)),
            ("assets/worlds/e.toml".to_owned(), Some(5)),
            ("assets/worlds/f.toml".to_owned(), Some(3)),
        ]
    );
    assert!(
        !findings.iter().any(
            |finding| finding.message.contains("trailing") || finding.message.contains("prose")
        ),
        "{findings:?}"
    );
    assert_eq!(
        located(&findings, "scenario-world-disallowed"),
        vec![("assets/scenarios.toml".to_owned(), Some(7))]
    );
    let disallowed: Vec<&WorkshopFinding> = findings
        .iter()
        .filter(|finding| finding.category == "member-disallowed")
        .collect();
    assert_eq!(disallowed.len(), 1, "{disallowed:?}");
    assert_eq!(disallowed[0].file, "assets/entities/nested/hull.toml");
    assert!(
        disallowed[0].message.contains("assets/worlds/a.toml"),
        "{}",
        disallowed[0].message
    );
    assert_eq!(
        (disallowed[0].severity.as_str(), disallowed[0].line),
        ("warning", None)
    );
    assert!(findings
        .iter()
        .all(|finding| finding.category == "member-disallowed" || finding.severity == "error"));
    let mut sorted = findings.clone();
    sorted.sort_by(|a, b| {
        (&a.file, a.line, &a.category, &a.message).cmp(&(&b.file, b.line, &b.category, &b.message))
    });
    assert_eq!(findings, sorted, "findings are deterministic");
    // A pack candidate leaves members to the archive gate.
    let mut pack = candidate.clone();
    pack.insert("scenarios.toml".into(), "[pack]\nformat = 1\n".into());
    assert!(located(&super::findings(&pack, &beneath), "member-disallowed").is_empty());
}

/// Every `.toml` and `.rhai` under `directory`, at any depth, keyed by its
/// forward-slash path: the member set a native Project workspace admits.
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

/// The shipped project exactly as `validate_project` sees it — every text
/// member under assets/, the audio catalogues, join codes and alternative
/// manifest included — yields no composition finding, so a Check of the
/// untouched project is quiet.
#[test]
fn shipped_content_has_no_composition_findings() {
    let mut candidate = BTreeMap::new();
    text_members_under("assets", &mut candidate);
    for expected in [
        "assets/scenarios.toml",
        "assets/scenarios.demo.toml",
        "assets/join/join-codes.toml",
        "assets/audio/room-ducking.toml",
    ] {
        assert!(
            candidate.contains_key(expected),
            "{expected} is a shipped member"
        );
    }
    let findings = findings(&candidate, &BTreeMap::new());
    assert!(findings.is_empty(), "{findings:?}");
    // The shipped default world's scripted layer is listed, resolved.
    let catalog = catalog(&candidate, &WorkshopDependencies::default());
    let default = catalog
        .worlds
        .iter()
        .find(|world| world.path == "assets/worlds/default.toml")
        .unwrap();
    assert!(default.script_refs.iter().any(|reference| {
        reference.path == "assets/worlds/reinforcements.toml"
            && reference.kind == "load"
            && reference.source == "inline-script"
            && reference.origin.as_deref() == Some("draft")
    }));
    assert_eq!(catalog.catalogue.len(), 2);
}

#[test]
fn new_world_source_is_a_world_the_runtime_reads() {
    let source = new_world_source("  Fresh start ").unwrap();
    assert_eq!(source, "[global]\ntitle = \"Fresh start\"\n");
    let parsed = crate::world::config::parse_world(&source).unwrap();
    assert_eq!(parsed.global.title.as_deref(), Some("Fresh start"));
    assert!(parsed.extra_worlds.is_empty());
    assert!(new_world_source("   ").is_err());
}

#[test]
fn scenario_catalogue_curates_ships_and_resolves_worlds_beneath() {
    let draft = draft();
    let beneath = dependencies().base_files;
    let catalogue = scenario_catalogue(&draft, &beneath);
    assert_eq!(
        catalogue
            .iter()
            .map(|entry| (entry.id.as_str(), entry.label.as_deref(), entry.ships.len()))
            .collect::<Vec<_>>(),
        vec![("root", Some("Root"), 1), ("based", Some("Base root"), 0)]
    );
    assert!(scenario_catalogue(&BTreeMap::new(), &beneath).is_empty());
}

/// A Test clears the composition rules of what it composes — the selected
/// root, its children and their sibling scripts — and nothing else in the
/// captured set, which lays the draft over its dependencies as one map.
#[test]
fn selection_findings_cover_the_selected_root_and_what_it_composes_only() {
    let files = files(&[
        (
            ROOT,
            "extra_worlds = [\"assets/worlds/child.toml\", \"assets/worlds/child.toml\"]\n[global]\n",
        ),
        (CHILD, "script = \"child.rhai\"\n[global]\n"),
        (
            "assets/worlds/child.rhai",
            "fn go(ctx) { ctx.effects.load_world(\"assets/worlds/nope.toml\"); }\n",
        ),
        (
            BASE_ROOT,
            "extra_worlds = [\"assets/worlds/gone.toml\"]\n[global]\n",
        ),
    ]);
    let located = |root: &str| {
        selection_findings(&files, root)
            .into_iter()
            .map(|finding| (finding.category, finding.file, finding.line))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        located(ROOT),
        vec![
            (
                "world-missing-load-reference".to_owned(),
                "assets/worlds/child.rhai".to_owned(),
                Some(1)
            ),
            (
                "extra-worlds-duplicate".to_owned(),
                ROOT.to_owned(),
                Some(1)
            ),
        ]
    );
    assert_eq!(
        located(BASE_ROOT),
        vec![(
            "extra-worlds-missing".to_owned(),
            BASE_ROOT.to_owned(),
            Some(1)
        )]
    );
    assert!(located(CHILD).len() == 1 && located("assets/worlds/elsewhere.toml").is_empty());
}
