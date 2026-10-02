use super::*;
const WORLD: &str = "assets/worlds/test.toml";
#[test]
fn nested_field_edit_retains_all_other_source_bytes() {
    let source = "# 文書\r\n[global] # keep\r\ntitle = 'Original' # retained\n[[entity]]\r\nposition = [1, 2, 3] # coordinates\r\ncustom = { other = 'unknown' }\n";
    let fields = fields(source, WORLD).unwrap();
    let coordinate = fields
        .iter()
        .find(|field| {
            field.path
                == vec![
                    Segment::Key("entity".into()),
                    Segment::Index(0),
                    Segment::Key("position".into()),
                    Segment::Index(1),
                ]
        })
        .unwrap();
    let result = patch(
        source,
        &Patch {
            document_path: WORLD.into(),
            path: coordinate.path.clone(),
            expected_source: source.into(),
            value_source: "8".into(),
        },
    )
    .unwrap();
    assert_eq!(result, source.replace("[1, 2, 3]", "[1, 8, 3]"));
    assert!(fields
        .iter()
        .any(|field| field.runtime_owned && field.descriptor.default_source.is_none()));
    assert!(fields
        .iter()
        .any(|field| !field.runtime_owned && field.source == "'unknown'"));
}
#[test]
fn stale_or_type_changing_edits_are_refused() {
    let source = "[global]\ntitle = 'Before'\n";
    let mut change = Patch {
        document_path: WORLD.into(),
        path: fields(source, WORLD).unwrap()[0].path.clone(),
        expected_source: source.into(),
        value_source: "10".into(),
    };
    assert!(patch(source, &change).is_err());
    change.value_source = "'After'".into();
    assert!(patch(&format!("{source}# external change\n"), &change).is_err());
    change.value_source = "'After'\nother = 'injected'".into();
    assert!(patch(source, &change).is_err());
}

#[test]
fn runtime_field_type_and_default_do_not_come_from_malformed_source() {
    let source = "[global]\ntitle = 4\nsim_tick_hz = 'invalid'\n";
    let descriptors = fields(source, WORLD).unwrap();
    assert_eq!(descriptors[0].descriptor.kind, "string");
    assert_eq!(descriptors[1].descriptor.kind, "float");
    assert_eq!(
        descriptors[1].descriptor.default_source,
        Some(
            crate::entities::config::GlobalConfig::default()
                .sim_tick_hz
                .to_string()
        )
    );
    let fixed = patch(
        source,
        &Patch {
            document_path: WORLD.into(),
            path: descriptors[0].path.clone(),
            expected_source: source.into(),
            value_source: "'Repaired'".into(),
        },
    )
    .unwrap();
    assert!(fixed.contains("title = 'Repaired'"));
}

// ── Structural edits (issue #1474) ────────────────────────────────────

const FACTION: &str = "assets/factions/rogue.toml";
const HULL: &str = "assets/entities/probe_hull.toml";
const ROGUE: &str = "eeeeeeee-5555-4555-8555-eeeeeeeeeeee";
const ALLIANCE: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";
const PIRATE: &str = "bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb";
const HARROW: &str = "cccccccc-3333-4333-8333-cccccccccccc";

fn faction_source() -> String {
    format!(
            "# Rogue traders\nuuid = \"{ROGUE}\"\nname = 'Rogue' # single quotes kept\nenemies = [\n    \"{ALLIANCE}\", # Alliance\n    \"{PIRATE}\", # Pirate\n]\nbanner = \"unknown extension\"\n\n[compliance]\nhold = \"refuse\"\n"
        )
}

fn hull_source() -> String {
    "class = \"cruiser\"\n\n[[station]]\nid = \"captain\"\nname = \"Captain\"\n\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[station.rating]]\nname = \"Simplified\"\nautomated_systems = [\"red-alert\"]\n\n[station.rating.ai_tuning]\ntorpedo_auto_fire = {}\n\n# ── Tactical ──\n[[station]]\nid = \"tactical\"\nname = \"Tactical\"\n\n[[station]]\nid = \"helm\"\nname = \"Helm\"\n\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[system]]\nid = \"red-alert\"\nkind = \"red_alert\"\nstation = \"captain\"\n".into()
}

fn key(text: &str) -> Segment {
    Segment::Key(text.into())
}

#[test]
fn workshop_slot_form_requests_preserve_exact_source() {
    for (name, request) in crate::core::codec::workshop_slot_edit_fixtures() {
        let source = &request.expected_source;
        let after = edit(source, &request).unwrap();
        if name == "label" {
            assert_eq!(
                after,
                source.replace(
                    "label='Original' # label tail\r\n",
                    "label=\"Changed\" # label tail\n"
                )
            );
        } else {
            assert!(after.starts_with(source));
            let world = crate::world::config::parse_world(&after).unwrap();
            assert_eq!(world.ship_slots[2].id, "wing");
            assert_eq!(
                world.ship_slots[2].ships[0].template_path,
                "assets/entities/base.toml"
            );
        }
    }
}

fn request(document_path: &str, source: &str, edits: Vec<Edit>) -> EditRequest {
    EditRequest {
        document_path: document_path.into(),
        expected_source: source.into(),
        edits,
    }
}

fn set(path: &[Segment], value_source: &str) -> Edit {
    Edit::Set {
        path: path.to_vec(),
        value_source: value_source.into(),
    }
}

fn put(path: &[Segment], value_source: &str) -> Edit {
    Edit::Put {
        path: path.to_vec(),
        value_source: value_source.into(),
    }
}

fn insert(path: &[Segment], index: usize, value_source: &str) -> Edit {
    Edit::Insert {
        path: path.to_vec(),
        index,
        value_source: value_source.into(),
    }
}

fn remove(path: &[Segment]) -> Edit {
    Edit::Remove {
        path: path.to_vec(),
    }
}

fn append(path: &[Segment], fields: &[(&str, &str)]) -> Edit {
    Edit::AppendTable {
        path: path.to_vec(),
        fields: fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

/// Every original line that carries none of the touched needles must
/// reappear byte-for-byte, ending included, and in the original order.
fn assert_untouched_lines_survive(original: &str, result: &str, touched: &[&str]) {
    let mut remaining = result;
    for (content, ending) in lines_with_endings(original) {
        if touched.iter().any(|needle| content.contains(needle)) {
            continue;
        }
        let line = format!("{content}{ending}");
        let at = remaining
            .find(&line)
            .unwrap_or_else(|| panic!("line {line:?} did not survive in:\n{result}"));
        remaining = &remaining[at + line.len()..];
    }
}

#[test]
fn set_replaces_one_scalar_and_keeps_every_other_byte_on_both_line_endings() {
    for source in [faction_source(), faction_source().replace('\n', "\r\n")] {
        let result = edit(
            &source,
            &request(FACTION, &source, vec![set(&[key("name")], "\"Renamed\"")]),
        )
        .unwrap();
        assert_eq!(result, source.replace("'Rogue'", "\"Renamed\""));
        assert_untouched_lines_survive(&source, &result, &["name = "]);
    }
}

#[test]
fn put_creates_a_root_key_after_the_last_root_value_and_updates_in_place() {
    let source = faction_source();
    let result = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![put(
                &[key("display_name")],
                "\"faction.rogue.display_name\"",
            )],
        ),
    )
    .unwrap();
    assert_eq!(
        result,
        source.replace(
            "banner = \"unknown extension\"\n",
            "banner = \"unknown extension\"\ndisplay_name = \"faction.rogue.display_name\"\n"
        )
    );
    let again = edit(
        &result,
        &request(
            FACTION,
            &result,
            vec![
                put(&[key("display_name")], "\"faction.rogue.other\""),
                put(&[key("compliance"), key("hold")], "\"comply\""),
                put(&[key("compliance"), key("decide_secs")], "7"),
            ],
        ),
    )
    .unwrap();
    assert_eq!(
        again,
        result
            .replace("faction.rogue.display_name", "faction.rogue.other")
            .replace(
                "hold = \"refuse\"\n",
                "hold = \"comply\"\ndecide_secs = 7\n"
            )
    );
}

#[test]
fn put_materialises_a_missing_parent_as_an_inline_table() {
    let source = hull_source();
    let helm_rule = [
        key("station"),
        Segment::Index(2),
        key("rating"),
        Segment::Index(0),
        key("ai_tuning"),
        key("torpedo_auto_fire"),
    ];
    let result = edit(
        &source,
        &request(HULL, &source, vec![put(&helm_rule, "{}")]),
    )
    .unwrap();
    assert_eq!(
            result,
            source.replace(
                "name = \"Std\"\nautomated_systems = []\n\n[[system]]",
                "name = \"Std\"\nautomated_systems = []\nai_tuning = { torpedo_auto_fire = {} }\n\n[[system]]"
            )
        );
    let removed = edit(&result, &request(HULL, &result, vec![remove(&helm_rule)])).unwrap();
    assert!(removed.contains("ai_tuning = {}\n"), "{removed}");
    let captain_rule = [
        key("station"),
        Segment::Index(0),
        key("rating"),
        Segment::Index(1),
        key("ai_tuning"),
        key("torpedo_auto_fire"),
    ];
    let cleared = edit(
        &source,
        &request(HULL, &source, vec![remove(&captain_rule)]),
    )
    .unwrap();
    assert_eq!(cleared, source.replace("torpedo_auto_fire = {}\n", ""));
    // The emptied `[station.rating.ai_tuning]` header stays a standard
    // table; the next rule goes back into it rather than being refused as
    // a value put over a table.
    let restored = edit(
        &cleared,
        &request(HULL, &cleared, vec![put(&captain_rule, "{}")]),
    )
    .unwrap();
    assert_eq!(restored, source);
}

#[test]
fn insert_at_the_end_of_a_multi_line_array_without_a_trailing_comma_keeps_the_bracket_line() {
    let tags = [key("tags")];
    let cases = [
        (
            "tags = [\n    \"A\",\n    \"B\"\n]\nafter = 1\n",
            "tags = [\n    \"A\",\n    \"B\",\n    \"C\"\n]\nafter = 1\n",
        ),
        (
            "tags = [\n    \"A\",\n    \"B\" # bye\n]\n",
            "tags = [\n    \"A\",\n    \"B\", # bye\n    \"C\"\n]\n",
        ),
        (
            "tags = [\n  \"A\",\n  \"B\"\n  ]\n",
            "tags = [\n  \"A\",\n  \"B\",\n  \"C\"\n  ]\n",
        ),
        (
            "tags = [\"A\", \"B\"\n]\n",
            "tags = [\"A\", \"B\",\n    \"C\"\n]\n",
        ),
    ];
    for (source, expected) in cases {
        for (source, expected) in [
            (source.to_owned(), expected.to_owned()),
            (source.replace('\n', "\r\n"), expected.replace('\n', "\r\n")),
        ] {
            let result = edit(
                &source,
                &request(WORLD, &source, vec![insert(&tags, 2, "\"C\"")]),
            )
            .unwrap();
            assert_eq!(result, expected);
            assert_untouched_lines_survive(&source, &result, &["\"B\""]);
        }
    }
}

#[test]
fn a_document_without_a_final_newline_keeps_that_convention() {
    for source in [
        format!("uuid = \"{ROGUE}\"\nname = \"X\""),
        format!("uuid = \"{ROGUE}\"\r\nname = \"X\""),
    ] {
        let ending = if source.contains('\r') { "\r\n" } else { "\n" };
        let renamed = edit(
            &source,
            &request(FACTION, &source, vec![set(&[key("name")], "\"Y\"")]),
        )
        .unwrap();
        assert_eq!(renamed, source.replace("\"X\"", "\"Y\""));
        let appended = edit(
            &source,
            &request(FACTION, &source, vec![put(&[key("display_name")], "\"d\"")]),
        )
        .unwrap();
        assert_eq!(appended, format!("{source}{ending}display_name = \"d\""));
    }
}

#[test]
fn remove_takes_a_rung_with_its_ai_tuning_table_and_leaves_the_next_station_intact() {
    let source = hull_source();
    let simplified = [
        key("station"),
        Segment::Index(0),
        key("rating"),
        Segment::Index(1),
    ];
    let result = edit(&source, &request(HULL, &source, vec![remove(&simplified)])).unwrap();
    assert_eq!(
            result,
            source.replace(
                "\n[[station.rating]]\nname = \"Simplified\"\nautomated_systems = [\"red-alert\"]\n\n[station.rating.ai_tuning]\ntorpedo_auto_fire = {}\n",
                ""
            )
        );
    assert!(result.contains("# ── Tactical ──\n[[station]]\nid = \"tactical\""));
    // A station's last rung leaves the station rung-less, and a rung can
    // be appended again afterwards.
    let helm = [key("station"), Segment::Index(2), key("rating")];
    let helm_std: Vec<Segment> = helm.iter().cloned().chain([Segment::Index(0)]).collect();
    let bare = edit(&source, &request(HULL, &source, vec![remove(&helm_std)])).unwrap();
    assert_eq!(
        bare,
        source.replace(
            "\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[system]]",
            "\n[[system]]"
        )
    );
    let again = edit(
        &bare,
        &request(
            HULL,
            &bare,
            vec![append(
                &helm,
                &[("name", "\"Std\""), ("automated_systems", "[]")],
            )],
        ),
    )
    .unwrap();
    assert_eq!(again, source);
    let missing: Vec<Segment> = helm.iter().cloned().chain([Segment::Index(1)]).collect();
    assert!(edit(&source, &request(HULL, &source, vec![remove(&missing)])).is_err());
}

#[test]
fn insert_keeps_a_multi_line_array_multi_line_and_comments_on_their_lines() {
    let source = faction_source();
    let enemies = [key("enemies")];
    let appended = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![insert(&enemies, 2, &format!("\"{HARROW}\""))],
        ),
    )
    .unwrap();
    assert_eq!(
        appended,
        source.replace(
            &format!("    \"{PIRATE}\", # Pirate\n]"),
            &format!("    \"{PIRATE}\", # Pirate\n    \"{HARROW}\",\n]")
        )
    );
    assert_untouched_lines_survive(&source, &appended, &[]);
    let first = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![insert(&enemies, 0, &format!("\"{HARROW}\""))],
        ),
    )
    .unwrap();
    assert_eq!(
        first,
        source.replace(
            "enemies = [\n",
            &format!("enemies = [\n    \"{HARROW}\",\n")
        )
    );
    let crlf = source.replace('\n', "\r\n");
    let appended = edit(
        &crlf,
        &request(
            FACTION,
            &crlf,
            vec![insert(&enemies, 2, &format!("\"{HARROW}\""))],
        ),
    )
    .unwrap();
    assert!(
        !appended.contains("\",\n"),
        "new text uses CRLF:\n{appended}"
    );
    assert_untouched_lines_survive(&crlf, &appended, &[]);
}

#[test]
fn insert_into_a_single_line_array_stays_on_one_line() {
    let source = format!("uuid = \"{ROGUE}\"\nname = \"Rogue\"\nenemies = [\"{ALLIANCE}\"]\n");
    let enemies = [key("enemies")];
    let appended = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![insert(&enemies, 1, &format!("\"{PIRATE}\""))],
        ),
    )
    .unwrap();
    assert_eq!(
        appended,
        source.replace(
            &format!("[\"{ALLIANCE}\"]"),
            &format!("[\"{ALLIANCE}\", \"{PIRATE}\"]")
        )
    );
    let first = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![insert(&enemies, 0, &format!("\"{PIRATE}\""))],
        ),
    )
    .unwrap();
    assert_eq!(
        first,
        source.replace(
            &format!("[\"{ALLIANCE}\"]"),
            &format!("[\"{PIRATE}\", \"{ALLIANCE}\"]")
        )
    );
    let empty = source.replace(&format!("[\"{ALLIANCE}\"]"), "[]");
    let filled = edit(
        &empty,
        &request(
            FACTION,
            &empty,
            vec![insert(&enemies, 0, &format!("\"{PIRATE}\""))],
        ),
    )
    .unwrap();
    assert_eq!(filled, empty.replace("[]", &format!("[\"{PIRATE}\"]")));
    // Padded brackets keep their padding around the whole list.
    let padded = source.replace(&format!("[\"{ALLIANCE}\"]"), &format!("[ \"{ALLIANCE}\" ]"));
    let last = edit(
        &padded,
        &request(
            FACTION,
            &padded,
            vec![insert(&enemies, 1, &format!("\"{PIRATE}\""))],
        ),
    )
    .unwrap();
    assert_eq!(
        last,
        padded.replace(
            &format!("[ \"{ALLIANCE}\" ]"),
            &format!("[ \"{ALLIANCE}\", \"{PIRATE}\" ]")
        )
    );
    let first = edit(
        &padded,
        &request(
            FACTION,
            &padded,
            vec![insert(&enemies, 0, &format!("\"{PIRATE}\""))],
        ),
    )
    .unwrap();
    assert_eq!(
        first,
        padded.replace(
            &format!("[ \"{ALLIANCE}\" ]"),
            &format!("[ \"{PIRATE}\", \"{ALLIANCE}\" ]")
        )
    );
    let blank = padded.replace(&format!("[ \"{ALLIANCE}\" ]"), "[ ]");
    let one = edit(
        &blank,
        &request(
            FACTION,
            &blank,
            vec![insert(&enemies, 0, &format!("\"{PIRATE}\""))],
        ),
    )
    .unwrap();
    assert_eq!(one, blank.replace("[ ]", &format!("[ \"{PIRATE}\" ]")));
    assert!(edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![insert(&enemies, 2, &format!("\"{PIRATE}\""))]
        )
    )
    .is_err());
}

#[test]
fn remove_takes_an_element_with_its_own_comment_and_no_other() {
    let source = faction_source();
    let first = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![remove(&[key("enemies"), Segment::Index(0)])],
        ),
    )
    .unwrap();
    assert_eq!(
        first,
        source.replace(&format!("    \"{ALLIANCE}\", # Alliance\n"), "")
    );
    let last = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![remove(&[key("enemies"), Segment::Index(1)])],
        ),
    )
    .unwrap();
    assert_eq!(
        last,
        source.replace(&format!("    \"{PIRATE}\", # Pirate\n"), "")
    );
    let none = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![
                remove(&[key("enemies"), Segment::Index(1)]),
                remove(&[key("enemies"), Segment::Index(0)]),
            ],
        ),
    )
    .unwrap();
    assert!(none.contains("enemies = []\n"), "{none}");
    assert_untouched_lines_survive(&source, &none, &["enemies", "# Alliance", "# Pirate", "]"]);
    let table_gone = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![remove(&[key("banner")]), remove(&[key("compliance")])],
        ),
    )
    .unwrap();
    assert_eq!(
        table_gone,
        source.replace(
            "banner = \"unknown extension\"\n\n[compliance]\nhold = \"refuse\"\n",
            ""
        )
    );
    assert!(edit(
        &source,
        &request(FACTION, &source, vec![remove(&[key("missing")])])
    )
    .is_err());
    assert!(edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![remove(&[key("enemies"), Segment::Index(2)])]
        )
    )
    .is_err());
}

/// A single-line array's padding belongs to the PAGE, not to the element
/// that happened to be last: `insert_element` already moves it onto a
/// newcomer at the end, and a removal has to hand it back the same way.
/// `toml_edit` stores the space before `]` as the last element's suffix, so
/// without this the array closes up tight (`[ "a"]`) — a byte-identity
/// break on the one shape the includes list is authored in by hand.
#[test]
fn removing_the_last_element_of_a_padded_single_line_array_keeps_the_padding() {
    let source = format!(
        "# Rogue traders\nuuid = \"{ROGUE}\"\nenemies = [ \"{ALLIANCE}\", \"{PIRATE}\" ]\n"
    );
    let last = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![remove(&[key("enemies"), Segment::Index(1)])],
        ),
    )
    .unwrap();
    assert_eq!(
        last,
        source.replace(&format!(", \"{PIRATE}\" ]"), " ]"),
        "the padding before ] survives"
    );
    // …and the symmetric insert puts it back exactly where it was, so the
    // two round-trip.
    assert_eq!(
        edit(
            &last,
            &request(
                FACTION,
                &last,
                vec![insert(&[key("enemies")], 1, &format!("\"{PIRATE}\""))],
            ),
        )
        .unwrap(),
        source
    );
    // Tight brackets stay tight: nothing is invented where there was no
    // padding.
    let tight = source.replace(
        &format!("[ \"{ALLIANCE}\", \"{PIRATE}\" ]"),
        &format!("[\"{ALLIANCE}\", \"{PIRATE}\"]"),
    );
    assert_eq!(
        edit(
            &tight,
            &request(
                FACTION,
                &tight,
                vec![remove(&[key("enemies"), Segment::Index(1)])],
            ),
        )
        .unwrap(),
        tight.replace(&format!(", \"{PIRATE}\"]"), "]")
    );
}

#[test]
fn append_table_lands_after_that_stations_last_rung_and_before_the_next_station() {
    let source = hull_source();
    let captain = [key("station"), Segment::Index(0), key("rating")];
    let result = edit(
        &source,
        &request(
            HULL,
            &source,
            vec![append(
                &captain,
                &[("name", "\"Novice\""), ("automated_systems", "[]")],
            )],
        ),
    )
    .unwrap();
    // After the last rung INCLUDING its `[station.rating.ai_tuning]`
    // sub-table, or the sub-table would attach to the new rung.
    assert_eq!(
            result,
            source.replace(
                "torpedo_auto_fire = {}\n\n# ── Tactical ──",
                "torpedo_auto_fire = {}\n\n[[station.rating]]\nname = \"Novice\"\nautomated_systems = []\n\n# ── Tactical ──"
            )
        );
    assert_untouched_lines_survive(&source, &result, &[]);
    let tactical = [key("station"), Segment::Index(1), key("rating")];
    let bare = edit(
        &source,
        &request(
            HULL,
            &source,
            vec![append(&tactical, &[("name", "\"Std\"")])],
        ),
    )
    .unwrap();
    assert_eq!(
            bare,
            source.replace(
                "name = \"Tactical\"\n\n[[station]]\nid = \"helm\"",
                "name = \"Tactical\"\n\n[[station.rating]]\nname = \"Std\"\n\n[[station]]\nid = \"helm\""
            )
        );
    assert!(
        edit(
            &source,
            &request(
                HULL,
                &source,
                vec![append(&captain, &[("name", "\"Std\"")])]
            )
        )
        .is_err(),
        "a duplicate rung name is refused at edit time"
    );
    assert!(edit(
        &source,
        &request(
            HULL,
            &source,
            vec![append(&captain, &[("automated_systems", "[]")])]
        )
    )
    .is_err());
}

#[test]
fn crlf_documents_get_crlf_new_lines_and_keep_every_other_line() {
    let source = hull_source().replace('\n', "\r\n");
    let captain = [key("station"), Segment::Index(0), key("rating")];
    let result = edit(
        &source,
        &request(
            HULL,
            &source,
            vec![
                append(
                    &captain,
                    &[("name", "\"Novice\""), ("automated_systems", "[]")],
                ),
                put(
                    &[key("station"), Segment::Index(1), key("visiting_rating")],
                    "\"Std\"",
                ),
            ],
        ),
    )
    .unwrap();
    assert!(
        !result.replace("\r\n", "").contains('\n'),
        "every line ends in CRLF:\n{result}"
    );
    assert!(
        result.contains("[[station.rating]]\r\nname = \"Novice\"\r\nautomated_systems = []\r\n")
    );
    assert!(result.contains("name = \"Tactical\"\r\nvisiting_rating = \"Std\"\r\n"));
    assert_untouched_lines_survive(&source, &result, &[]);
    let mixed = hull_source().replacen('\n', "\r\n", 3);
    let result = edit(
        &mixed,
        &request(
            HULL,
            &mixed,
            vec![append(&captain, &[("name", "\"Novice\"")])],
        ),
    )
    .unwrap();
    assert_untouched_lines_survive(&mixed, &result, &[]);
    assert!(
        result.contains("[[station.rating]]\nname = \"Novice\"\n"),
        "{result}"
    );
}

#[test]
fn a_refused_edit_in_a_group_applies_nothing_and_stale_or_injected_text_is_refused() {
    let source = faction_source();
    let refused = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![
                set(&[key("name")], "\"Renamed\""),
                insert(&[key("enemies")], 0, "\"not-a-uuid\""),
            ],
        ),
    );
    assert!(refused.is_err());
    assert!(edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![put(&[key("compliance"), key("hold")], "\"maybe\"")]
        )
    )
    .is_err());
    assert!(edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![set(&[key("name")], "\"A\"\nextra = 1")]
        )
    )
    .is_err());
    assert!(edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![set(&[key("name")], "\"A\" # comment\n[table]")]
        )
    )
    .is_err());
    assert!(edit(
        &source,
        &request(FACTION, &source, vec![set(&[key("enemies")], "\"scalar\"")])
    )
    .is_err());
    assert!(edit(
        &source,
        &request(FACTION, &source, vec![set(&[key("name")], "4")])
    )
    .is_err());
    assert!(edit(
        &format!("{source}# moved\n"),
        &request(FACTION, &source, vec![set(&[key("name")], "\"A\"")])
    )
    .is_err());
    let hull = hull_source();
    let rung_name = [
        key("station"),
        Segment::Index(0),
        key("rating"),
        Segment::Index(0),
        key("name"),
    ];
    assert!(edit(&hull, &request(HULL, &hull, vec![set(&rung_name, "\"\"")])).is_err());
    let commented = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![set(&[key("name")], "\"A\" # trailing")],
        ),
    )
    .unwrap();
    assert_eq!(
        commented,
        source.replace("'Rogue'", "\"A\""),
        "a trailing comment does not ride in"
    );
}

#[test]
fn a_whole_form_apply_is_one_reparseable_result() {
    let source = faction_source();
    let result = edit(
        &source,
        &request(
            FACTION,
            &source,
            vec![
                set(&[key("name")], "\"Renamed\""),
                remove(&[key("enemies"), Segment::Index(1)]),
                insert(&[key("enemies")], 1, &format!("\"{HARROW}\"")),
                put(&[key("compliance"), key("divert")], "\"refuse\""),
            ],
        ),
    )
    .unwrap();
    let parsed = crate::ai::faction::parse_faction_config(&result).unwrap();
    assert_eq!(parsed.name, "Renamed");
    assert_eq!(
        parsed
            .enemies
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec![ALLIANCE.to_owned(), HARROW.to_owned()]
    );
    let compliance = parsed.compliance.unwrap();
    assert_eq!(compliance.hold, crate::civilian::OrderResponse::Refuse);
    assert_eq!(compliance.divert, crate::civilian::OrderResponse::Refuse);
    assert!(result.contains("# Rogue traders\n"));
    assert!(result.contains("# Alliance\n"));
    assert!(result.contains("banner = \"unknown extension\"\n"));
}

#[test]
fn manifest_extension_keys_do_not_inherit_an_unrelated_world_schema() {
    let source = "[global]\ntitle = 4\n";
    let field = fields(source, "scenarios.toml").unwrap().remove(0);
    assert!(!field.runtime_owned);
    assert_eq!(field.descriptor.kind, "integer");
    assert_eq!(
        patch(
            source,
            &Patch {
                document_path: "scenarios.toml".into(),
                path: field.path,
                expected_source: source.into(),
                value_source: "5".into()
            }
        )
        .unwrap(),
        source.replace('4', "5")
    );
}
