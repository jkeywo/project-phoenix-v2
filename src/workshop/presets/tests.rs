use super::*;
use crate::workshop::document::{Edit, Segment};
use crate::workshop::WorkshopDependencyPack;

const WORLD: &str = "assets/worlds/desk.toml";
const CHILD: &str = "assets/worlds/desk_child.toml";
const BASE_WORLD: &str = "assets/worlds/base_desk.toml";
const HULL: &str = "assets/entities/hull.toml";

/// A world the runtime accepts, with every preset and widget facet authored
/// once. The line of every value is the index of its line in this list plus
/// one, so a catalog assertion names the line an author would read.
fn world() -> String {
    [
        // `extra_worlds` is a ROOT key, so it comes before the first table.
        "extra_worlds = [\"assets/worlds/desk_child.toml\"]", // 1
        "",                                                   // 2
        "[global]",                                           // 3
        "title = \"world.desk.title\"",                       // 4
        "",                                                   // 5
        "[[entity]]",                                         // 6
        "template_path = \"assets/entities/hull.toml\"",      // 7
        "name = \"world.desk.escort\"",                       // 8
        "",                                                   // 9
        "[[gm_role_preset]]",                                 // 10
        "id = \"tactical\"",                                  // 11
        "label = \"world.desk.preset.tactical\"",             // 12
        "panels = [\"gm-map-panel\", \"gm-activity\"]",       // 13
        "quick_actions = [\"gm-session-pause\"]",             // 14
        "contacts = [\"world.desk.escort\", \"world.desk.relay\"]", // 15
        "note = \"kept\"",                                    // 16
        "",                                                   // 17
        "[[gm_role_preset.widget]]",                          // 18
        "id = \"urgent\"",                                    // 19
        "type = \"attention\"",                               // 20
        "label = \"world.desk.widget.urgent\"",               // 21
        "band = \"urgent\"",                                  // 22
        "category = \"pending_comms\"",                       // 23
        "ship = \"world.desk.escort\"",                       // 24
        "",                                                   // 25
        "[[gm_role_preset.widget]]",                          // 26
        "id = \"levers\"",                                    // 27
        "type = \"actions\"",                                 // 28
        "label = \"world.desk.widget.levers\"",               // 29
        "actions = [\"gm-session-pause\", \"gm-session-resume\"]", // 30
        "",                                                   // 31
        "[[gm_role_preset.widget]]",                          // 32
        "id = \"brief\"",                                     // 33
        "type = \"note\"",                                    // 34
        "label = \"world.desk.widget.brief\"",                // 35
        "text = \"world.desk.note.brief\"",                   // 36
        "",                                                   // 37
        "[[gm_role_preset]]",                                 // 38
        "id = \"observer\"",                                  // 39
        "label = \"world.desk.preset.observer\"",             // 40
        "",                                                   // 41
        "[[gm_role_preset.widget]]",                          // 42
        "id = \"seats\"",                                     // 43
        "type = \"workload\"",                                // 44
        "label = \"world.desk.widget.seats\"",                // 45
        "ship = \"world.desk.relay\"",                        // 46
        "",
    ]
    .join("\n")
}

/// The composed child: `world.desk.relay` is declared HERE, so the parent's
/// contact and its workload widget's ship resolve through composition.
fn child() -> String {
    [
        "[[entity]]",
        "template_path = \"assets/entities/hull.toml\"",
        "name = \"world.desk.relay\"",
        "",
    ]
    .join("\n")
}

fn files() -> BTreeMap<String, String> {
    BTreeMap::from([
        (WORLD.to_owned(), world()),
        (CHILD.to_owned(), child()),
        (HULL.to_owned(), "class = \"lancer\"\n".to_owned()),
    ])
}

/// One read-only base world, so the catalog's world list carries an origin
/// other than `draft`.
fn dependencies() -> WorkshopDependencies {
    WorkshopDependencies {
        base_files: BTreeMap::from([(
            BASE_WORLD.to_owned(),
            "[global]\ntitle = \"base\"\n".to_owned(),
        )]),
        ..Default::default()
    }
}

fn request(path: &str, source: &str, edits: Vec<Edit>) -> EditRequest {
    EditRequest {
        document_path: path.to_owned(),
        expected_source: source.to_owned(),
        edits,
    }
}

fn key(name: &str) -> Segment {
    Segment::Key(name.to_owned())
}

/// `gm_role_preset[preset].widget[widget].<key>`, the address the panel edits.
fn widget_path(preset: usize, widget: usize, field: &str) -> Vec<Segment> {
    vec![
        key(PRESETS_KEY),
        Segment::Index(preset),
        key(WIDGETS_KEY),
        Segment::Index(widget),
        key(field),
    ]
}

fn preset_path(preset: usize, field: &str) -> Vec<Segment> {
    vec![key(PRESETS_KEY), Segment::Index(preset), key(field)]
}

fn located(findings: &[WorkshopFinding]) -> Vec<(String, Option<usize>, String)> {
    findings
        .iter()
        .map(|finding| {
            (
                finding.category.clone(),
                finding.line,
                finding.severity.clone(),
            )
        })
        .collect()
}

// ── Catalog ───────────────────────────────────────────────────────────────────

#[test]
fn the_fixture_world_is_one_the_runtime_reads() {
    let parsed = parse_world(&world()).expect("the fixture must be a world the runtime accepts");
    assert_eq!(parsed.gm_role_presets.len(), 2);
    assert_eq!(parsed.gm_role_presets[0].widget.len(), 3);
}

#[test]
fn the_catalog_reads_every_authored_value_with_its_exact_line() {
    let catalog = catalog(&files(), &dependencies(), WORLD);
    assert_eq!(catalog.path, WORLD);
    assert_eq!(catalog.origin, ORIGIN_DRAFT);
    assert!(catalog.findings.is_empty(), "{:?}", catalog.findings);
    assert_eq!(catalog.presets.len(), 2);

    let tactical = &catalog.presets[0];
    assert_eq!((tactical.index, tactical.id.as_str()), (0, "tactical"));
    assert_eq!(tactical.id_line, 11);
    assert_eq!(tactical.label, "world.desk.preset.tactical");
    assert_eq!(tactical.label_line, 12);
    assert_eq!(
        tactical
            .panels
            .iter()
            .map(|entry| (entry.index, entry.value.as_str(), entry.line))
            .collect::<Vec<_>>(),
        vec![(0, "gm-map-panel", 13), (1, "gm-activity", 13)]
    );
    assert_eq!(
        tactical
            .quick_actions
            .iter()
            .map(|entry| (entry.value.as_str(), entry.line))
            .collect::<Vec<_>>(),
        vec![("gm-session-pause", 14)]
    );
    // The second contact is declared by the CHILD world, and composition is
    // what makes it known.
    assert_eq!(
        tactical
            .contacts
            .iter()
            .map(|entry| (entry.value.as_str(), entry.line, entry.known))
            .collect::<Vec<_>>(),
        vec![
            ("world.desk.escort", 15, true),
            ("world.desk.relay", 15, true)
        ]
    );
    // An unknown key is preserved and reported, never dropped.
    assert_eq!(tactical.unknown_keys, vec!["note".to_owned()]);

    let attention = &tactical.widgets[0];
    assert_eq!((attention.id.as_str(), attention.id_line), ("urgent", 19));
    assert_eq!(
        (attention.kind.as_str(), attention.kind_line),
        ("attention", 20)
    );
    assert_eq!(attention.label_line, 21);
    let band = attention.band.as_ref().expect("band");
    assert_eq!((band.value.as_str(), band.line), ("urgent", 22));
    let category = attention.category.as_ref().expect("category");
    assert_eq!(
        (category.value.as_str(), category.line),
        ("pending_comms", 23)
    );
    let ship = attention.ship.as_ref().expect("ship");
    assert_eq!(
        (ship.value.as_str(), ship.line, ship.known),
        ("world.desk.escort", 24, true)
    );
    assert!(attention.actions.is_empty());
    assert!(attention.text.is_none());
    assert!(attention.unknown_keys.is_empty());

    let levers = &tactical.widgets[1];
    assert_eq!(
        levers
            .actions
            .iter()
            .map(|action| (
                action.index,
                action.value.as_str(),
                action.line,
                action.known
            ))
            .collect::<Vec<_>>(),
        vec![
            (0, "gm-session-pause", 30, true),
            (1, "gm-session-resume", 30, true)
        ]
    );
    let brief = &tactical.widgets[2];
    let text = brief.text.as_ref().expect("text");
    assert_eq!(
        (text.value.as_str(), text.line),
        ("world.desk.note.brief", 36)
    );

    let observer = &catalog.presets[1];
    assert_eq!(observer.id, "observer");
    let seats = &observer.widgets[0];
    assert_eq!(seats.kind, "workload");
    assert!(seats.ship.as_ref().expect("ship").known);
}

#[test]
fn the_catalog_choices_are_the_runtimes_own_vocabularies() {
    let catalog = catalog(&files(), &dependencies(), WORLD);
    assert_eq!(
        catalog.choices.widget_types,
        GM_WIDGET_TYPES.map(str::to_owned).to_vec()
    );
    assert_eq!(
        catalog.choices.widget_actions,
        GM_WIDGET_ACTION_IDS.map(str::to_owned).to_vec()
    );
    // Every band and category the panel offers is one the runtime's own parser
    // reads back, and there are as many as the runtime publishes.
    for band in &catalog.choices.bands {
        assert!(
            GmAttentionBand::from_authored(band).is_some(),
            "offered band {band:?} is not one the runtime reads"
        );
    }
    assert_eq!(catalog.choices.bands.len(), 3);
    for category in &catalog.choices.categories {
        assert!(
            GmAttentionCategory::from_authored(category).is_some(),
            "offered category {category:?} is not one the runtime reads"
        );
    }
    assert_eq!(
        catalog.choices.categories.len(),
        GmAttentionCategory::all().len()
    );
    // `ship` and `contacts` name entities, and composition supplies the child's.
    assert_eq!(
        catalog.choices.entities,
        vec![
            "world.desk.escort".to_owned(),
            "world.desk.relay".to_owned()
        ]
    );
    // The browser owns the panel and quick-action vocabularies (D2); nothing
    // here offers one, and nothing here warns about one.
    let encoded = crate::core::codec::encode_workshop_preset_catalog(&catalog).unwrap();
    assert!(!encoded.contains("gm-map-panel\",\"origin"), "{encoded}");
    assert!(!encoded.contains("not-drawn"), "{encoded}");
}

#[test]
fn the_catalog_lists_the_world_members_a_preset_may_be_authored_in() {
    let catalog = catalog(&files(), &dependencies(), WORLD);
    assert_eq!(
        catalog
            .worlds
            .iter()
            .map(|world| (world.path.as_str(), world.origin.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (WORLD, ORIGIN_DRAFT),
            (CHILD, ORIGIN_DRAFT),
            (BASE_WORLD, ORIGIN_BASE),
        ]
    );
}

#[test]
fn a_pack_world_is_listed_read_only_under_its_origin() {
    let dependencies = WorkshopDependencies {
        packs: vec![WorkshopDependencyPack {
            id: "aurora".into(),
            manifest_toml: String::new(),
            files: BTreeMap::from([(
                "assets/worlds/pack_desk.toml".to_owned(),
                "[global]\ntitle = \"pack\"\n\n[[gm_role_preset]]\nid = \"pack\"\nlabel = \"l\"\n"
                    .to_owned(),
            )]),
            assets: BTreeMap::new(),
        }],
        ..Default::default()
    };
    let catalog = catalog(&files(), &dependencies, "assets/worlds/pack_desk.toml");
    assert_eq!(catalog.origin, "pack:aurora");
    assert_eq!(catalog.presets.len(), 1);
    assert!(catalog
        .worlds
        .iter()
        .any(|world| world.origin == "pack:aurora"));
}

#[test]
fn a_path_that_is_not_a_world_member_is_said_so_rather_than_shown_blank() {
    for path in [HULL, "assets/worlds/missing.toml"] {
        let catalog = catalog(&files(), &dependencies(), path);
        assert!(catalog.origin.is_empty(), "{path}");
        assert!(catalog.presets.is_empty(), "{path}");
        assert!(
            catalog
                .findings
                .iter()
                .any(|finding| finding.category == "unknown-document" && finding.file == path),
            "{path}: {:?}",
            catalog.findings
        );
    }
}

// ── Findings ──────────────────────────────────────────────────────────────────

/// Every rule broken once, each on a line of its own.
fn broken() -> String {
    [
        "[[gm_role_preset]]",                         // 1
        "id = \"all\"",                               // 2
        "label = \"\"",                               // 3
        "contacts = [\"ghost\"]",                     // 4
        "",                                           // 5
        "[[gm_role_preset.widget]]",                  // 6
        "id = \"\"",                                  // 7
        "type = \"attention\"",                       // 8
        "label = \"\"",                               // 9
        "band = \"critical\"",                        // 10
        "category = \"gossip\"",                      // 11
        "ship = \"nobody\"",                          // 12
        "",                                           // 13
        "[[gm_role_preset.widget]]",                  // 14
        "id = \"dupe\"",                              // 15
        "type = \"note\"",                            // 16
        "label = \"l\"",                              // 17
        "text = \"ok.text\"",                         // 18
        "band = \"urgent\"",                          // 19
        "",                                           // 20
        "[[gm_role_preset.widget]]",                  // 21
        "id = \"dupe\"",                              // 22
        "type = \"wat\"",                             // 23
        "label = \"l\"",                              // 24
        "",                                           // 25
        "[[gm_role_preset]]",                         // 26
        "id = \"all\"",                               // 27
        "label = \"l\"",                              // 28
        "",                                           // 29
        "[[gm_role_preset.widget]]",                  // 30
        "id = \"levers\"",                            // 31
        "type = \"actions\"",                         // 32
        "label = \"l\"",                              // 33
        "actions = [\"gm-session-pause\", \"nope\"]", // 34
        "",
    ]
    .join("\n")
}

#[test]
fn every_rule_is_an_error_finding_at_its_own_line() {
    let candidate = BTreeMap::from([(WORLD.to_owned(), broken())]);
    let findings = findings(&candidate, &BTreeMap::new());
    assert_eq!(
        located(&findings),
        vec![
            ("preset-reserved-id".to_owned(), Some(2), "error".to_owned()),
            ("preset-empty-label".to_owned(), Some(3), "error".to_owned()),
            (
                "preset-unknown-contact".to_owned(),
                Some(4),
                "error".to_owned()
            ),
            ("widget-empty-id".to_owned(), Some(7), "error".to_owned()),
            ("widget-empty-label".to_owned(), Some(9), "error".to_owned()),
            (
                "widget-unknown-band".to_owned(),
                Some(10),
                "error".to_owned()
            ),
            (
                "widget-unknown-category".to_owned(),
                Some(11),
                "error".to_owned()
            ),
            (
                "widget-unknown-ship".to_owned(),
                Some(12),
                "error".to_owned()
            ),
            (
                "widget-key-on-wrong-type".to_owned(),
                Some(19),
                "error".to_owned()
            ),
            (
                "widget-duplicate-id".to_owned(),
                Some(22),
                "error".to_owned()
            ),
            (
                "widget-unknown-type".to_owned(),
                Some(23),
                "error".to_owned()
            ),
            (
                "preset-reserved-id".to_owned(),
                Some(27),
                "error".to_owned()
            ),
            (
                "widget-unknown-action".to_owned(),
                Some(34),
                "error".to_owned()
            ),
        ],
        "{findings:#?}"
    );
    assert!(findings.iter().all(|finding| finding.file == WORLD));
}

#[test]
fn an_empty_preset_id_and_the_other_preset_level_rules_are_located() {
    let candidate = BTreeMap::from([(
        WORLD.to_owned(),
        [
            "[[gm_role_preset]]",
            "id = \"\"",
            "label = \"l\"",
            "",
            "[[gm_role_preset]]",
            "id = \"twice\"",
            "label = \"l\"",
            "",
            "[[gm_role_preset]]",
            "id = \"twice\"",
            "label = \"l\"",
            "",
        ]
        .join("\n"),
    )]);
    assert_eq!(
        located(&findings(&candidate, &BTreeMap::new())),
        vec![
            ("preset-empty-id".to_owned(), Some(2), "error".to_owned()),
            (
                "preset-duplicate-id".to_owned(),
                Some(10),
                "error".to_owned()
            ),
        ]
    );
}

#[test]
fn the_note_and_actions_rules_the_runtime_refuses_are_located_too() {
    let candidate = BTreeMap::from([(
        WORLD.to_owned(),
        [
            "[[gm_role_preset]]",                                     // 1
            "id = \"p\"",                                             // 2
            "label = \"l\"",                                          // 3
            "",                                                       // 4
            "[[gm_role_preset.widget]]",                              // 5
            "id = \"empty-note\"",                                    // 6
            "type = \"note\"",                                        // 7
            "label = \"l\"",                                          // 8
            "",                                                       // 9
            "[[gm_role_preset.widget]]",                              // 10
            "id = \"prose\"",                                         // 11
            "type = \"note\"",                                        // 12
            "label = \"l\"",                                          // 13
            "text = \"<b>hi</b>\"",                                   // 14
            "",                                                       // 15
            "[[gm_role_preset.widget]]",                              // 16
            "id = \"bare\"",                                          // 17
            "type = \"actions\"",                                     // 18
            "label = \"l\"",                                          // 19
            "",                                                       // 20
            "[[gm_role_preset.widget]]",                              // 21
            "id = \"twice\"",                                         // 22
            "type = \"actions\"",                                     // 23
            "label = \"l\"",                                          // 24
            "actions = [\"gm-session-pause\", \"gm-session-pause\"]", // 25
            "",
        ]
        .join("\n"),
    )]);
    assert_eq!(
        located(&findings(&candidate, &BTreeMap::new())),
        vec![
            ("widget-empty-text".to_owned(), Some(5), "error".to_owned()),
            (
                "widget-invalid-text".to_owned(),
                Some(14),
                "error".to_owned()
            ),
            (
                "widget-empty-actions".to_owned(),
                Some(16),
                "error".to_owned()
            ),
            (
                "widget-duplicate-action".to_owned(),
                Some(25),
                "error".to_owned()
            ),
        ]
    );
}

/// The detail a finding and a refusal carry is the RUNTIME's sentence, not a
/// paraphrase of it: the same widget through `parse_world` says the same words.
#[test]
fn a_widget_findings_detail_is_the_runtimes_own_sentence() {
    for (broken, category) in [
        ("band = \"critical\"", "widget-unknown-band"),
        ("category = \"gossip\"", "widget-unknown-category"),
        ("ship = \"\"", "widget-unknown-ship"),
    ] {
        let source = [
            "[[gm_role_preset]]",
            "id = \"tactical\"",
            "label = \"l\"",
            "",
            "[[gm_role_preset.widget]]",
            "id = \"w\"",
            "type = \"attention\"",
            "label = \"wl\"",
            broken,
            "",
        ]
        .join("\n");
        let refused = parse_world(&source)
            .expect_err("the runtime must refuse this world")
            .to_string();
        let candidate = BTreeMap::from([(WORLD.to_owned(), source)]);
        let findings = findings(&candidate, &BTreeMap::new());
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].category, category);
        assert_eq!(
            findings[0].message, refused,
            "{category} must carry the runtime's own sentence"
        );
    }
}

/// The four PRESET-level sentences this module writes itself repeat
/// `parse_world`'s own words (see the module header), and are the one place a
/// reword in `world::config` could drift away from the Workshop unnoticed —
/// every widget sentence is the runtime's own string and gets its guard for
/// free. They are compared clause by clause, because the middles differ ON
/// PURPOSE: the runtime's duplicate messages name BOTH offending indices, while
/// a finding points at one line and names the one entry sitting on it.
#[test]
fn the_preset_sentences_are_pinned_to_parse_worlds_own_words() {
    /// What the reader reads first: the offender, up to the first `:` or `;`.
    fn claim(sentence: &str) -> &str {
        let end = sentence.find([':', ';']).unwrap_or(sentence.len());
        sentence[..end].trim()
    }
    /// Why the rule exists: everything after the last `;`.
    fn reason(sentence: &str) -> &str {
        sentence.rsplit(';').next().unwrap_or(sentence).trim()
    }
    let preset = |id: &str| {
        vec![
            "[[gm_role_preset]]".to_owned(),
            format!("id = \"{id}\""),
            "label = \"l\"".to_owned(),
            String::new(),
        ]
    };
    let note = |id: &str| {
        vec![
            "[[gm_role_preset.widget]]".to_owned(),
            format!("id = \"{id}\""),
            "type = \"note\"".to_owned(),
            "label = \"l\"".to_owned(),
            "text = \"ok.text\"".to_owned(),
            String::new(),
        ]
    };
    let mut duplicate_presets = preset("twice");
    duplicate_presets.extend(preset("twice"));
    let mut duplicate_widgets = preset("p");
    duplicate_widgets.extend(note("w"));
    duplicate_widgets.extend(note("w"));
    for (category, lines) in [
        ("preset-empty-id", preset("")),
        ("preset-reserved-id", preset(GM_ROLE_PRESET_ALL_ID)),
        ("preset-duplicate-id", duplicate_presets),
        ("widget-duplicate-id", duplicate_widgets),
    ] {
        let source = lines.join("\n");
        let refused = parse_world(&source)
            .expect_err("the runtime must refuse this world")
            .to_string();
        let candidate = BTreeMap::from([(WORLD.to_owned(), source)]);
        let findings = findings(&candidate, &BTreeMap::new());
        assert_eq!(findings.len(), 1, "{category}: {findings:#?}");
        assert_eq!(findings[0].category, category);
        assert_eq!(
            claim(&findings[0].message),
            claim(&refused),
            "{category} must name the offender in the runtime's own words"
        );
        assert_eq!(
            reason(&findings[0].message),
            reason(&refused),
            "{category} must give the runtime's own reason"
        );
    }
}

#[test]
fn an_unknown_type_and_a_misplaced_key_carry_the_runtimes_sentence_too() {
    for (kind, extra, category) in [
        ("wat", "", "widget-unknown-type"),
        ("workload", "band = \"urgent\"", "widget-key-on-wrong-type"),
        ("note", "ship = \"x\"", "widget-key-on-wrong-type"),
    ] {
        let mut lines = vec![
            "[[gm_role_preset]]".to_owned(),
            "id = \"tactical\"".to_owned(),
            "label = \"l\"".to_owned(),
            String::new(),
            "[[gm_role_preset.widget]]".to_owned(),
            "id = \"w\"".to_owned(),
            format!("type = \"{kind}\""),
            "label = \"wl\"".to_owned(),
            "text = \"a.b\"".to_owned(),
        ];
        if !extra.is_empty() {
            lines.push(extra.to_owned());
        }
        lines.push(String::new());
        let source = lines.join("\n");
        let refused = parse_world(&source)
            .expect_err("the runtime must refuse this world")
            .to_string();
        let candidate = BTreeMap::from([(WORLD.to_owned(), source)]);
        let findings = findings(&candidate, &BTreeMap::new());
        assert!(
            findings
                .iter()
                .any(|finding| finding.category == category && finding.message == refused),
            "{kind}/{extra}: {findings:#?}"
        );
    }
}

#[test]
fn the_shipped_content_yields_no_preset_findings() {
    let mut candidate = BTreeMap::new();
    text_members_under("assets", &mut candidate);
    for expected in [
        "assets/worlds/combat_test.toml",
        "assets/worlds/probe_gm_widgets.toml",
    ] {
        assert!(
            candidate.contains_key(expected),
            "{expected} is a shipped world with authored presets"
        );
    }
    let findings = findings(&candidate, &BTreeMap::new());
    assert!(findings.is_empty(), "{findings:#?}");
    // And the shipped widget probe reads back with every facet located.
    let catalog = catalog(
        &candidate,
        &WorkshopDependencies::default(),
        "assets/worlds/probe_gm_widgets.toml",
    );
    assert_eq!(catalog.presets.len(), 2);
    let narrative = &catalog.presets[1];
    let ship = narrative.widgets[0].ship.as_ref().expect("ship");
    assert_eq!(ship.value, "world.probe_gm_widgets.escort");
    assert!(ship.known, "a shipped ship reference must resolve");
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

// ── Edits ─────────────────────────────────────────────────────────────────────

#[test]
fn an_accepted_edit_changes_one_value_and_nothing_else() {
    let files = files();
    let edited = compose(
        &files,
        &dependencies(),
        &request(
            WORLD,
            &world(),
            vec![Edit::Set {
                path: widget_path(0, 0, "band"),
                value_source: "\"background\"".into(),
            }],
        ),
    )
    .expect("a band the runtime reads is accepted");
    assert_eq!(
        edited,
        world().replace("band = \"urgent\"", "band = \"background\"")
    );
    // Comments, the unknown key and every other byte survive.
    assert!(edited.contains("note = \"kept\""));
    parse_world(&edited).expect("the edited world still loads");
}

#[test]
fn a_new_widget_of_each_type_can_be_appended_in_one_history_entry() {
    let files = files();
    let edited = compose(
        &files,
        &dependencies(),
        &request(
            WORLD,
            &world(),
            vec![Edit::AppendTable {
                path: vec![key(PRESETS_KEY), Segment::Index(1), key(WIDGETS_KEY)],
                fields: vec![
                    ("id".into(), "\"added\"".into()),
                    ("type".into(), "\"note\"".into()),
                    ("label".into(), "\"world.desk.widget.added\"".into()),
                    ("text".into(), "\"world.desk.note.added\"".into()),
                ],
            }],
        ),
    )
    .expect("a complete note widget is accepted");
    let parsed = parse_world(&edited).expect("the edited world loads");
    assert_eq!(parsed.gm_role_presets[1].widget.len(), 2);
    assert_eq!(parsed.gm_role_presets[1].widget[1].id, "added");
}

/// Every edit SHAPE the browser panel plans, in one group over one world: an
/// array a preset does not carry yet (`put`), an element into one it does
/// (`insert`) and one out of another (`remove`), a widget scalar replaced
/// (`set`) and one cleared (`remove`), a whole widget table appended
/// (`append_table`) with a key then put on it, and a whole widget table removed.
///
/// The panel's planner emits exactly these, and an Apply is ONE group however
/// many it holds — so a shape the exact-source owner could not apply would be a
/// press that refuses for a reason no preset rule describes. Proved here rather
/// than trusted, because the two halves of this form were built apart.
#[test]
fn the_panels_whole_edit_vocabulary_lands_as_one_group() {
    let files = files();
    let widgets = |preset: usize| vec![key(PRESETS_KEY), Segment::Index(preset), key(WIDGETS_KEY)];
    let element = |preset: usize, field: &str, slot: usize| {
        vec![
            key(PRESETS_KEY),
            Segment::Index(preset),
            key(field),
            Segment::Index(slot),
        ]
    };
    let edited = compose(
        &files,
        &dependencies(),
        &request(
            WORLD,
            &world(),
            vec![
                Edit::Put {
                    path: preset_path(1, "panels"),
                    value_source: "[\"gm-map-panel\"]".into(),
                },
                Edit::Insert {
                    path: preset_path(0, "quick_actions"),
                    index: 1,
                    value_source: "\"gm-session-resume\"".into(),
                },
                Edit::Remove {
                    path: element(0, "panels", 1),
                },
                Edit::Set {
                    path: widget_path(0, 2, "text"),
                    value_source: "\"world.desk.note.moved\"".into(),
                },
                Edit::Remove {
                    path: widget_path(0, 0, "band"),
                },
                Edit::AppendTable {
                    path: widgets(1),
                    fields: vec![
                        ("id".into(), "\"added\"".into()),
                        ("type".into(), "\"attention\"".into()),
                        ("label".into(), "\"world.desk.widget.added\"".into()),
                    ],
                },
                // A key the appended widget did not carry, which is the `put`
                // the planner emits whenever the document held no such key.
                Edit::Put {
                    path: widget_path(1, 1, "band"),
                    value_source: "\"background\"".into(),
                },
                Edit::Remove {
                    path: element(0, WIDGETS_KEY, 1),
                },
            ],
        ),
    )
    .expect("every shape the panel plans is one the exact-source owner applies");
    let parsed = parse_world(&edited).expect("the edited world loads");
    let ids = |preset: usize| {
        parsed.gm_role_presets[preset]
            .widget
            .iter()
            .map(|widget| widget.id.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(parsed.gm_role_presets[0].panels, vec!["gm-map-panel"]);
    assert_eq!(
        parsed.gm_role_presets[0].quick_actions,
        vec!["gm-session-pause", "gm-session-resume"]
    );
    assert_eq!(ids(0), vec!["urgent", "brief"]);
    assert_eq!(parsed.gm_role_presets[0].widget[0].band, None);
    assert_eq!(
        parsed.gm_role_presets[0].widget[1].text.as_deref(),
        Some("world.desk.note.moved")
    );
    assert_eq!(parsed.gm_role_presets[1].panels, vec!["gm-map-panel"]);
    assert_eq!(ids(1), vec!["seats", "added"]);
    assert_eq!(
        parsed.gm_role_presets[1].widget[1].band.as_deref(),
        Some("background")
    );
    // The unknown key and every other authored byte survive the whole group.
    assert!(edited.contains("note = \"kept\""));
    assert!(edited.contains("title = \"world.desk.title\""));

    // A whole preset, widgets and all, is the one remaining shape: the panel's
    // Remove plans it at the entry's own index.
    let removed = compose(
        &files,
        &dependencies(),
        &request(
            WORLD,
            &world(),
            vec![Edit::Remove {
                path: vec![key(PRESETS_KEY), Segment::Index(1)],
            }],
        ),
    )
    .expect("a whole preset may be removed");
    let parsed = parse_world(&removed).expect("the world loads without it");
    assert_eq!(parsed.gm_role_presets.len(), 1);
    assert_eq!(parsed.gm_role_presets[0].id, "tactical");
    assert!(!removed.contains("seats"));
}

/// Every refusal category, and the source untouched in every one of them.
#[test]
fn every_rule_the_edit_introduces_is_refused_with_the_source_untouched() {
    let files = files();
    let cases: Vec<(&str, Vec<Edit>)> = vec![
        (
            "preset-reserved-id",
            vec![Edit::Set {
                path: preset_path(0, "id"),
                value_source: "\"all\"".into(),
            }],
        ),
        (
            "preset-duplicate-id",
            vec![Edit::Set {
                path: preset_path(1, "id"),
                value_source: "\"tactical\"".into(),
            }],
        ),
        (
            "preset-empty-id",
            vec![Edit::Set {
                path: preset_path(0, "id"),
                value_source: "\"\"".into(),
            }],
        ),
        (
            "preset-empty-label",
            vec![Edit::Set {
                path: preset_path(0, "label"),
                value_source: "\"  \"".into(),
            }],
        ),
        (
            "preset-unknown-contact",
            vec![Edit::Insert {
                path: preset_path(0, "contacts"),
                index: 2,
                value_source: "\"ghost\"".into(),
            }],
        ),
        (
            "widget-duplicate-id",
            vec![Edit::Set {
                path: widget_path(0, 1, "id"),
                value_source: "\"urgent\"".into(),
            }],
        ),
        (
            "widget-empty-id",
            vec![Edit::Set {
                path: widget_path(0, 0, "id"),
                value_source: "\"\"".into(),
            }],
        ),
        (
            "widget-empty-label",
            vec![Edit::Set {
                path: widget_path(0, 0, "label"),
                value_source: "\"\"".into(),
            }],
        ),
        (
            "widget-unknown-type",
            vec![Edit::Set {
                path: widget_path(0, 0, "type"),
                value_source: "\"gauge\"".into(),
            }],
        ),
        (
            "widget-key-on-wrong-type",
            vec![Edit::Put {
                path: widget_path(0, 2, "band"),
                value_source: "\"urgent\"".into(),
            }],
        ),
        (
            "widget-unknown-band",
            vec![Edit::Set {
                path: widget_path(0, 0, "band"),
                value_source: "\"critical\"".into(),
            }],
        ),
        (
            "widget-unknown-category",
            vec![Edit::Set {
                path: widget_path(0, 0, "category"),
                value_source: "\"gossip\"".into(),
            }],
        ),
        (
            "widget-unknown-ship",
            vec![Edit::Set {
                path: widget_path(0, 0, "ship"),
                value_source: "\"nobody\"".into(),
            }],
        ),
        (
            "widget-unknown-action",
            vec![Edit::Insert {
                path: widget_path(0, 1, "actions"),
                index: 2,
                value_source: "\"nope\"".into(),
            }],
        ),
        (
            "widget-empty-text",
            vec![Edit::Set {
                path: widget_path(0, 2, "text"),
                value_source: "\"\"".into(),
            }],
        ),
        (
            "widget-invalid-text",
            vec![Edit::Set {
                path: widget_path(0, 2, "text"),
                value_source: "\"<b>hi</b>\"".into(),
            }],
        ),
        (
            "widget-duplicate-action",
            vec![Edit::Set {
                path: vec![
                    key(PRESETS_KEY),
                    Segment::Index(0),
                    key(WIDGETS_KEY),
                    Segment::Index(1),
                    key("actions"),
                    Segment::Index(1),
                ],
                value_source: "\"gm-session-pause\"".into(),
            }],
        ),
    ];
    for (category, edits) in cases {
        let error = compose(&files, &dependencies(), &request(WORLD, &world(), edits))
            .expect_err("{category} must be refused");
        assert!(
            error.starts_with(&format!("{category}: ")),
            "expected {category}, got: {error}"
        );
        assert_eq!(
            files.get(WORLD).map(String::as_str),
            Some(world().as_str()),
            "{category} must leave the source untouched"
        );
    }
}

#[test]
fn an_empty_actions_list_is_refused_as_a_card_with_nothing_on_it() {
    let files = files();
    let error = compose(
        &files,
        &dependencies(),
        &request(
            WORLD,
            &world(),
            vec![Edit::Remove {
                path: widget_path(0, 1, "actions"),
            }],
        ),
    )
    .expect_err("an actions widget with no actions is refused");
    assert!(error.starts_with("widget-empty-actions: "), "{error}");
}

#[test]
fn a_document_the_draft_does_not_carry_is_refused_as_unknown() {
    let files = files();
    for path in [HULL, "assets/worlds/missing.toml"] {
        let error = compose(
            &files,
            &dependencies(),
            &request(
                path,
                files.get(path).map(String::as_str).unwrap_or_default(),
                vec![Edit::Set {
                    path: preset_path(0, "id"),
                    value_source: "\"x\"".into(),
                }],
            ),
        )
        .expect_err("only a draft world member may be edited");
        assert!(error.starts_with("unknown-document: "), "{path}: {error}");
    }
}

/// A reorder is `set` edits on the swapped slots, so an existing violation
/// MOVES. It is matched by its offending value rather than its index, so the
/// reorder is accepted and the violation is still reported at its new line.
#[test]
fn a_reorder_that_moves_an_existing_violation_is_accepted() {
    let source = [
        "[[gm_role_preset]]",        // 1
        "id = \"first\"",            // 2
        "label = \"l\"",             // 3
        "",                          // 4
        "[[gm_role_preset.widget]]", // 5
        "id = \"w\"",                // 6
        "type = \"attention\"",      // 7
        "label = \"wl\"",            // 8
        "band = \"critical\"",       // 9
        "",                          // 10
        "[[gm_role_preset]]",        // 11
        "id = \"second\"",           // 12
        "label = \"l\"",             // 13
        "",
    ]
    .join("\n");
    let files = BTreeMap::from([(WORLD.to_owned(), source.clone())]);
    let edited = compose(
        &files,
        &WorkshopDependencies::default(),
        &request(
            WORLD,
            &source,
            vec![
                Edit::Set {
                    path: preset_path(0, "id"),
                    value_source: "\"second\"".into(),
                },
                Edit::Set {
                    path: preset_path(1, "id"),
                    value_source: "\"first\"".into(),
                },
            ],
        ),
    )
    .expect("a reorder must not read as a new violation");
    assert!(edited.contains("id = \"second\"\nlabel"));
    let candidate = BTreeMap::from([(WORLD.to_owned(), edited)]);
    assert_eq!(
        located(&findings(&candidate, &BTreeMap::new())),
        vec![(
            "widget-unknown-band".to_owned(),
            Some(9),
            "error".to_owned()
        )]
    );
}

#[test]
fn a_hand_broken_draft_can_be_repaired_one_edit_at_a_time() {
    let source = [
        "[[gm_role_preset]]",
        "id = \"p\"",
        "label = \"l\"",
        "",
        "[[gm_role_preset.widget]]",
        "id = \"a\"",
        "type = \"attention\"",
        "label = \"wl\"",
        "band = \"critical\"",
        "",
        "[[gm_role_preset.widget]]",
        "id = \"b\"",
        "type = \"attention\"",
        "label = \"wl\"",
        "band = \"sudden\"",
        "",
    ]
    .join("\n");
    let files = BTreeMap::from([(WORLD.to_owned(), source.clone())]);
    let dependencies = WorkshopDependencies::default();
    // Repairing one of two existing violations is accepted, although the other
    // is still there.
    let edited = compose(
        &files,
        &dependencies,
        &request(
            WORLD,
            &source,
            vec![Edit::Set {
                path: widget_path(0, 0, "band"),
                value_source: "\"urgent\"".into(),
            }],
        ),
    )
    .expect("a repair must not be refused for the violation it does not touch");
    assert!(edited.contains("band = \"sudden\""));
    // A SECOND copy of a violation the member already carried is new.
    let error = compose(
        &files,
        &dependencies,
        &request(
            WORLD,
            &source,
            vec![Edit::Set {
                path: widget_path(0, 1, "band"),
                value_source: "\"critical\"".into(),
            }],
        ),
    )
    .expect_err("a second copy of an existing violation is introduced");
    assert!(error.starts_with("widget-unknown-band: "), "{error}");
}

#[test]
fn a_stale_expected_source_is_refused_before_any_rule_is_read() {
    let files = files();
    let error = compose(
        &files,
        &dependencies(),
        &request(
            WORLD,
            "[[gm_role_preset]]\n",
            vec![Edit::Set {
                path: preset_path(0, "id"),
                value_source: "\"x\"".into(),
            }],
        ),
    )
    .expect_err("an optimistic concurrency failure is still a refusal");
    assert!(error.contains("changed"), "{error}");
}

/// A contact or a ship a DEPENDENCY's world declares resolves, because the
/// reference set is candidate ∪ dependencies exactly as Check resolves it.
#[test]
fn a_reference_a_dependency_declares_resolves() {
    let mut files = files();
    files.remove(CHILD);
    let dependencies = WorkshopDependencies {
        base_files: BTreeMap::from([(CHILD.to_owned(), child())]),
        ..Default::default()
    };
    let through_dependency = catalog(&files, &dependencies, WORLD);
    assert!(
        through_dependency.findings.is_empty(),
        "{:?}",
        through_dependency.findings
    );
    assert!(through_dependency.presets[0].contacts[1].known);
    // And the same reference with the child gone is an error at its line.
    let alone = catalog(&files, &WorkshopDependencies::default(), WORLD);
    assert_eq!(
        located(&alone.findings),
        vec![
            (
                "preset-unknown-contact".to_owned(),
                Some(15),
                "error".to_owned()
            ),
            (
                "widget-unknown-ship".to_owned(),
                Some(46),
                "error".to_owned()
            ),
        ],
        "{:#?}",
        alone.findings
    );
}

// ── New preset ────────────────────────────────────────────────────────────────

#[test]
fn new_preset_source_is_a_block_the_runtime_reads() {
    let source = new_preset_source("  tactical ", " world.desk.preset.tactical ").unwrap();
    assert_eq!(
        source,
        "[[gm_role_preset]]\nid = \"tactical\"\nlabel = \"world.desk.preset.tactical\"\n"
    );
    let parsed = parse_world(&source).expect("the new block loads on its own");
    assert_eq!(parsed.gm_role_presets.len(), 1);
    assert_eq!(parsed.gm_role_presets[0].id, "tactical");
    assert!(parsed.gm_role_presets[0].widget.is_empty());
    assert!(new_preset_source("  ", "label").is_err());
    assert!(new_preset_source("id", " ").is_err());
    let reserved = new_preset_source(GM_ROLE_PRESET_ALL_ID, "label")
        .expect_err("the reserved id is refused in the form");
    assert!(reserved.contains(GM_ROLE_PRESET_ALL_ID), "{reserved}");
}

// ── The runtime as the judge ───────────────────────────────────────────────────

/// The ownership table is the runtime's, asked rather than copied. This pins
/// the answers so a change to `GmRolePresetWidget::validate` shows up here
/// rather than as a panel offering a key that does nothing.
#[test]
fn which_type_owns_which_facet_is_the_runtimes_answer() {
    let owned = |kind: &str| {
        [
            Facet::BandCategory,
            Facet::Ship,
            Facet::Actions,
            Facet::Text,
        ]
        .into_iter()
        .filter(|facet| owns(kind, *facet))
        .map(Facet::key)
        .collect::<Vec<_>>()
    };
    assert_eq!(owned("attention"), vec!["band/category", "ship"]);
    assert_eq!(owned("workload"), vec!["ship"]);
    assert_eq!(owned("actions"), vec!["actions"]);
    assert_eq!(owned("note"), vec!["text"]);
    // Every type this build draws owns at least one facet, so a new type
    // cannot arrive with nothing the panel can offer for it.
    for kind in GM_WIDGET_TYPES {
        assert!(!owned(kind).is_empty(), "{kind} owns no facet");
    }
}

/// A name a SCRIPT mints is authored correctly, and the gate that refuses an
/// export must not call it unknown. The one case a Workshop author answers for
/// by hand — a name assembled from a variable — is still reported, so the rule
/// keeps its teeth where it can see.
#[test]
fn a_ship_or_contact_a_script_spawns_is_known_and_one_nobody_mints_is_not() {
    const SCRIPTED: &str = "assets/worlds/scripted.toml";
    const SIBLING: &str = "assets/worlds/scripted.rhai";
    let world = [
        "script = \"scripted.rhai\"",
        "",
        "[global]",
        "title = \"Scripted\"",
        "",
        "[[gm_role_preset]]",
        "id = \"watch\"",
        "label = \"world.scripted.preset.watch\"",
        "contacts = [\"watcher\", \"nobody\"]",
        "",
        "[[gm_role_preset.widget]]",
        "id = \"seats\"",
        "type = \"workload\"",
        "label = \"world.scripted.widget.seats\"",
        "ship = \"escort\"",
        "",
    ]
    .join("\n");
    // Two names this world's script mints, and one it assembles, which nothing
    // structural can see.
    let sibling = concat!(
        "fn arm(ctx) {\n",
        "    ctx.effects.spawn_entity(#{ template_path: \"assets/entities/hull.toml\", name: \"watcher\" });\n",
        "    ctx.effects.spawn_entity(#{ template_path: \"assets/entities/hull.toml\", name: \"escort\" });\n",
        "    ctx.effects.spawn_entity(#{ template_path: \"assets/entities/hull.toml\", name: mint(ctx) });\n",
        "}\n"
    );
    let candidate = BTreeMap::from([
        (SCRIPTED.to_owned(), world),
        (SIBLING.to_owned(), sibling.to_owned()),
    ]);
    let findings = findings(&candidate, &BTreeMap::new());
    assert_eq!(
        located(&findings),
        vec![(
            "preset-unknown-contact".to_owned(),
            Some(9),
            "error".to_owned()
        )],
        "{findings:?}"
    );
}
