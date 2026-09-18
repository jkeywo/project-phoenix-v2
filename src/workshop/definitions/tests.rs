use super::*;
use crate::workshop::WorkshopDependencyPack;

const ALLIANCE: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";
const PIRATE: &str = "bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb";
const ROGUE: &str = "eeeeeeee-5555-4555-8555-eeeeeeeeeeee";
const NOBODY: &str = "ffffffff-6666-4666-8666-ffffffffffff";

const ROGUE_PATH: &str = "assets/factions/rogue.toml";
const HULL_PATH: &str = "assets/entities/probe_hull.toml";

fn rogue() -> String {
    format!(
        "# Rogue traders\nuuid = \"{ROGUE}\"\nname = \"Rogue\"\ndisplay_name = \"faction.rogue.display_name\"\nenemies = [\n    \"{ALLIANCE}\", # Alliance\n    \"{NOBODY}\", # nobody\n]\nbanner = \"unknown extension\"\n\n[compliance]\nhold = \"refuse\"\nack_secs = 5\n"
    )
}

fn hull() -> String {
    format!(
        "class = \"cruiser\"\nfaction = \"{ALLIANCE}\"\n\n[[station]]\nid = \"captain\"\nname = \"Captain\"\nhuman_seeking = true\nvisiting_rating = \"Std\"\n\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[station.rating]]\nname = \"Simplified\"\nautomated_systems = [\"red-alert\", \"phaser-fore\", \"ghost\"]\n\n[station.rating.ai_tuning]\ntorpedo_auto_fire = {{}}\n\n[[station]]\nid = \"tactical\"\nname = \"Tactical\"\n\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[system]]\nid = \"red-alert\"\nkind = \"red_alert\"\nstation = \"captain\"\n\n[[system]]\nid = \"phaser-fore\"\nkind = \"phaser_bank\"\nstation = \"tactical\"\n"
    )
}

fn base() -> WorkshopDependencies {
    WorkshopDependencies {
        base_files: BTreeMap::from([
            (
                "assets/factions/alliance.toml".into(),
                include_str!("../../../assets/factions/alliance.toml").into(),
            ),
            (
                "assets/factions/pirate.toml".into(),
                include_str!("../../../assets/factions/pirate.toml").into(),
            ),
        ]),
        base_assets: BTreeMap::new(),
        packs: Vec::new(),
    }
}

fn files(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect()
}

fn located(findings: &[WorkshopFinding], category: &str) -> Vec<(String, Option<usize>)> {
    findings
        .iter()
        .filter(|finding| finding.category == category)
        .map(|finding| (finding.file.clone(), finding.line))
        .collect()
}

#[test]
fn faction_catalog_carries_exact_lines_resolved_names_and_runtime_choices() {
    let catalog = catalog(&files(&[(ROGUE_PATH, &rogue())]), &base());
    assert_eq!(
        catalog
            .factions
            .iter()
            .map(|faction| (faction.path.as_str(), faction.origin.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("assets/factions/alliance.toml", "base"),
            ("assets/factions/pirate.toml", "base"),
            (ROGUE_PATH, "draft"),
        ]
    );
    let draft = &catalog.factions[2];
    assert_eq!(draft.uuid.as_deref(), Some(ROGUE));
    assert_eq!(draft.uuid_line, Some(2));
    assert_eq!(draft.name.as_deref(), Some("Rogue"));
    assert_eq!(draft.name_line, Some(3));
    assert_eq!(
        draft.display_name.as_deref(),
        Some("faction.rogue.display_name")
    );
    assert_eq!(draft.display_name_line, Some(4));
    assert_eq!(
        draft
            .enemies
            .iter()
            .map(|enemy| (enemy.uuid.as_str(), enemy.line, enemy.name.as_deref()))
            .collect::<Vec<_>>(),
        vec![(ALLIANCE, 6, Some("Alliance")), (NOBODY, 7, None)]
    );
    let compliance = draft.compliance.as_ref().unwrap();
    assert_eq!(compliance.len(), 2);
    assert_eq!(compliance["hold"].source, "\"refuse\"");
    assert_eq!(compliance["hold"].line, 12);
    assert_eq!(compliance["ack_secs"].source, "5");
    assert_eq!(compliance["ack_secs"].line, 13);
    assert_eq!(draft.unknown_keys, vec!["banner".to_owned()]);
    let alliance = &catalog.factions[0];
    assert!(alliance.compliance.is_none());
    assert_eq!(alliance.enemies[0].name.as_deref(), Some("Pirate"));

    assert_eq!(
        catalog
            .choices
            .factions
            .iter()
            .map(|choice| (
                choice.name.as_str(),
                choice.uuid.as_str(),
                choice.origin.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("Alliance", ALLIANCE, "base"),
            ("Pirate", PIRATE, "base"),
            ("Rogue", ROGUE, "draft"),
        ]
    );
    assert_eq!(catalog.choices.order_responses, vec!["comply", "refuse"]);
    assert_eq!(catalog.choices.ai_rules, vec!["torpedo_auto_fire"]);
    let defaults = ComplianceDisposition::default();
    assert_eq!(catalog.defaults.compliance.ack_secs, defaults.ack_secs);
    assert_eq!(
        catalog.defaults.compliance.decide_secs,
        defaults.decide_secs
    );
    assert_eq!(catalog.defaults.compliance.hold, "comply");
    assert_eq!(catalog.defaults.compliance.divert, "comply");
    assert_eq!(catalog.defaults.compliance.dock, "comply");
    assert_eq!(
        located(&catalog.findings, "faction-unknown-enemy"),
        vec![(ROGUE_PATH.to_owned(), Some(7))]
    );
}

#[test]
fn pack_factions_are_listed_under_their_pack_and_the_draft_wins_a_path() {
    let mut dependencies = base();
    dependencies.packs.push(WorkshopDependencyPack {
        id: "extra".into(),
        manifest_toml: String::new(),
        files: BTreeMap::from([
            (
                "assets/factions/rogue.toml".into(),
                format!("uuid = \"{ROGUE}\"\nname = \"PackRogue\"\n"),
            ),
            (
                "assets/factions/nobody.toml".into(),
                format!("uuid = \"{NOBODY}\"\nname = \"Nobody\"\n"),
            ),
        ]),
        assets: BTreeMap::new(),
    });
    let catalog = catalog(&files(&[(ROGUE_PATH, &rogue())]), &dependencies);
    let nobody = catalog
        .factions
        .iter()
        .find(|faction| faction.path == "assets/factions/nobody.toml")
        .unwrap();
    assert_eq!(nobody.origin, "pack:extra");
    let rogue = catalog
        .factions
        .iter()
        .find(|faction| faction.path == ROGUE_PATH)
        .unwrap();
    assert_eq!(rogue.origin, "draft");
    assert_eq!(rogue.name.as_deref(), Some("Rogue"));
    assert_eq!(rogue.enemies[1].name.as_deref(), Some("Nobody"));
    assert!(catalog.findings.is_empty(), "{:?}", catalog.findings);
    assert!(catalog
        .choices
        .factions
        .iter()
        .any(|choice| choice.name == "Rogue" && choice.origin == "draft"));
}

#[test]
fn hull_catalog_lists_stations_owned_systems_rungs_and_rules_with_lines() {
    let catalog = catalog(
        &files(&[
            (HULL_PATH, &hull()),
            (
                "assets/entities/fragment.toml",
                "[collider]\nshape = \"Ball\"\nradius = 1.0\n",
            ),
        ]),
        &base(),
    );
    assert_eq!(
        catalog.hulls.len(),
        1,
        "a fragment without stations is not a hull"
    );
    let hull = &catalog.hulls[0];
    assert_eq!(hull.path, HULL_PATH);
    assert_eq!(hull.origin, "draft");
    let captain = &hull.stations[0];
    assert_eq!(captain.id, "captain");
    assert_eq!(captain.name, "Captain");
    assert_eq!(captain.line, 4);
    assert!(captain.human_seeking);
    let visiting = captain.visiting_rating.as_ref().unwrap();
    assert_eq!((visiting.source.as_str(), visiting.line), ("\"Std\"", 8));
    assert_eq!(
        captain
            .systems
            .iter()
            .map(|system| (system.id.as_str(), system.kind.as_str(), system.line))
            .collect::<Vec<_>>(),
        vec![("red-alert", "red_alert", 29)]
    );
    assert_eq!(
        captain
            .ratings
            .iter()
            .map(|rating| (rating.index, rating.name.as_str(), rating.line))
            .collect::<Vec<_>>(),
        vec![(0, "Std", 10), (1, "Simplified", 14)]
    );
    let simplified = &captain.ratings[1];
    assert_eq!(
        simplified
            .automated_systems
            .iter()
            .map(|system| (system.id.as_str(), system.line, system.owned))
            .collect::<Vec<_>>(),
        vec![
            ("red-alert", 16, true),
            ("phaser-fore", 16, false),
            ("ghost", 16, false)
        ]
    );
    assert_eq!(
        simplified
            .ai_tuning
            .iter()
            .map(|rule| (rule.rule.as_str(), rule.line))
            .collect::<Vec<_>>(),
        vec![("torpedo_auto_fire", 19)]
    );
    let tactical = &hull.stations[1];
    assert_eq!((tactical.id.as_str(), tactical.line), ("tactical", 21));
    assert!(!tactical.human_seeking);
    assert!(tactical.visiting_rating.is_none());
    assert_eq!(tactical.systems[0].id, "phaser-fore");
    assert_eq!(tactical.ratings[0].line, 25);
    assert_eq!(
        located(&catalog.findings, "rating-unowned-system"),
        vec![(HULL_PATH.to_owned(), Some(16))]
    );
    assert_eq!(
        located(&catalog.findings, "rating-unknown-system"),
        vec![(HULL_PATH.to_owned(), Some(16))]
    );
}

#[test]
fn every_faction_finding_category_reports_its_exact_line() {
    let candidate = files(&[
        (
            "assets/factions/a_dup.toml",
            &format!("uuid = \"{ALLIANCE}\"\nname = \"Alliance\"\n"),
        ),
        (
            "assets/factions/b_self.toml",
            &format!("uuid = \"{ROGUE}\"\nname = \"Rogue\"\nenemies = [\n  \"{ROGUE}\",\n  \"{NOBODY}\",\n  \"bad\",\n]\n"),
        ),
        (
            "assets/factions/c_bad.toml",
            "uuid = \"nope\"\nname = \"Bad\"\n",
        ),
        (
            "assets/factions/d_twin.toml",
            &format!("name = \"Rogue\"\nuuid = \"{PIRATE}\"\n"),
        ),
        (
            "assets/factions/e_twin.toml",
            &format!("uuid = \"{PIRATE}\"\nname = \"Twin\"\n"),
        ),
    ]);
    let beneath = base().base_files;
    let findings = findings(&candidate, &beneath);
    // A candidate duplicating a read-only file carries the finding; the base
    // file cannot change. Two candidates sharing a key: the second by path.
    assert_eq!(
        located(&findings, "faction-duplicate-uuid"),
        vec![
            ("assets/factions/a_dup.toml".to_owned(), Some(1)),
            ("assets/factions/d_twin.toml".to_owned(), Some(2)),
            ("assets/factions/e_twin.toml".to_owned(), Some(1)),
        ]
    );
    assert_eq!(
        located(&findings, "faction-duplicate-name"),
        vec![
            ("assets/factions/a_dup.toml".to_owned(), Some(2)),
            ("assets/factions/d_twin.toml".to_owned(), Some(1)),
        ]
    );
    assert_eq!(
        located(&findings, "faction-self-enemy"),
        vec![("assets/factions/b_self.toml".to_owned(), Some(4))]
    );
    assert_eq!(
        located(&findings, "faction-unknown-enemy"),
        vec![("assets/factions/b_self.toml".to_owned(), Some(5))]
    );
    assert_eq!(
        located(&findings, "faction-invalid-uuid"),
        vec![
            ("assets/factions/b_self.toml".to_owned(), Some(6)),
            ("assets/factions/c_bad.toml".to_owned(), Some(1)),
        ]
    );
    assert!(findings.iter().all(|finding| finding.severity == "error"));
    let mut sorted = findings.clone();
    sorted.sort_by(|a, b| {
        (&a.file, a.line, &a.category, &a.message).cmp(&(&b.file, b.line, &b.category, &b.message))
    });
    assert_eq!(findings, sorted, "findings are deterministic");
}

#[test]
fn entity_world_and_rating_findings_mirror_the_runtime_refusals_with_lines() {
    let broken_hull = hull()
        .replace(
            "[[station.rating]]\nname = \"Simplified\"",
            "[[station.rating]]\nname = \"Std\"",
        )
        .replace("visiting_rating = \"Std\"", "visiting_rating = \"Expert\"")
        .replace(
            "id = \"tactical\"\nname = \"Tactical\"",
            "id = \"tactical\"\nname = \"Tactical\"\nhuman_seeking = true",
        )
        .replace(ALLIANCE, NOBODY);
    let candidate = files(&[
        (HULL_PATH, &broken_hull),
        (
            "assets/worlds/probe.toml",
            "[global]\ntitle = \"Probe\"\n[[trigger]]\nid = \"arm\"\n[[trigger.action]]\ntype = \"add_faction_enemy\"\nfaction = \"Harrow\"\nenemy = \"Alliance\"\n[[trigger.action]]\ntype = \"remove_faction_enemy\"\nfaction = \"Alliance\"\nenemy = \"Ghost\"\nscript = '''\n// ctx.effects.add_faction_enemy(\"Commented\", \"Out\");\nfn on_world_loaded(ctx) {\n    ctx.effects.add_faction_enemy(\"Alliance\", \"Nobody\");\n    ctx.effects.remove_faction_enemy(name, \"Alliance\");\n}\n'''\n",
        ),
        (
            "assets/worlds/probe.rhai",
            "fn arm(ctx) {\n    ctx.effects.add_faction_enemy(\"Pirate\", \"Phantom\");\n}\n",
        ),
    ]);
    let findings = findings(&candidate, &base().base_files);
    assert_eq!(
        located(&findings, "entity-unknown-faction"),
        vec![(HULL_PATH.to_owned(), Some(2))]
    );
    assert_eq!(
        located(&findings, "rating-duplicate-name"),
        vec![(HULL_PATH.to_owned(), Some(14))]
    );
    assert_eq!(
        located(&findings, "rating-unknown-system"),
        vec![(HULL_PATH.to_owned(), Some(16))]
    );
    assert_eq!(
        located(&findings, "rating-unowned-system"),
        vec![(HULL_PATH.to_owned(), Some(16))]
    );
    assert_eq!(
        located(&findings, "station-unknown-visiting-rating"),
        vec![(HULL_PATH.to_owned(), Some(8))]
    );
    assert_eq!(
        located(&findings, "station-missing-visiting-rating"),
        vec![(HULL_PATH.to_owned(), Some(21))]
    );
    assert_eq!(
        located(&findings, "world-unknown-faction"),
        vec![
            ("assets/worlds/probe.rhai".to_owned(), Some(2)),
            ("assets/worlds/probe.toml".to_owned(), Some(7)),
            ("assets/worlds/probe.toml".to_owned(), Some(12)),
            ("assets/worlds/probe.toml".to_owned(), Some(16)),
        ]
    );
    let unowned = findings
        .iter()
        .find(|finding| finding.category == "rating-unowned-system")
        .unwrap();
    assert_eq!(
        unowned.message,
        ShipConfigError::RatingReferencesUnownedSystem {
            station: StationId("captain".into()),
            rating: "Std".into(),
            system: SystemId("phaser-fore".into()),
            owner: Some(StationId("tactical".into())),
        }
        .to_string(),
        "the message is the runtime's own refusal"
    );
    let duplicate = findings
        .iter()
        .find(|finding| finding.category == "rating-duplicate-name")
        .unwrap();
    assert!(duplicate.message.contains("DuplicateRatingName"));
}

#[test]
fn a_visiting_rating_or_host_order_without_human_seeking_is_a_finding_at_its_line() {
    let tactical = hull().replace(
        "id = \"tactical\"\nname = \"Tactical\"",
        "id = \"tactical\"\nname = \"Tactical\"\nvisiting_rating = \"Std\"\nhost_order = [\"captain\"]",
    );
    let reported = findings(&files(&[(HULL_PATH, &tactical)]), &base().base_files);
    assert_eq!(
        located(&reported, "station-visiting-rating-without-human-seeking"),
        vec![(HULL_PATH.to_owned(), Some(24))]
    );
    assert_eq!(
        located(&reported, "station-host-order-without-human-seeking"),
        vec![(HULL_PATH.to_owned(), Some(25))]
    );
    let refusal = reported
        .iter()
        .find(|finding| finding.category == "station-visiting-rating-without-human-seeking")
        .unwrap();
    assert_eq!(
        refusal.message,
        ShipConfigError::HostOrderWithoutHumanSeeking {
            station: StationId("tactical".into()),
        }
        .to_string()
    );
    // The captain seats a human, so its own visiting rating is not reported,
    // and the shipped hull without either key is clean.
    assert!(reported.iter().all(
        |finding| !finding.category.ends_with("without-human-seeking") || finding.line >= Some(24)
    ));
    assert!(located(
        &findings(&files(&[(HULL_PATH, &hull())]), &base().base_files),
        "station-visiting-rating-without-human-seeking"
    )
    .is_empty());
}

#[test]
fn a_uuid_declared_beneath_the_draft_twice_resolves_like_the_registry_and_is_warned_about() {
    // Whichever file name sorts first, the pack beats the base — the order
    // the runtime registry inserts in — and the choice label agrees with the
    // name a draft's enemy entry resolves to.
    for (base_path, pack_path) in [
        ("assets/factions/a.toml", "assets/factions/z.toml"),
        ("assets/factions/z.toml", "assets/factions/a.toml"),
    ] {
        let mut dependencies = base();
        dependencies.base_files.insert(
            base_path.into(),
            format!("uuid = \"{NOBODY}\"\nname = \"Older\"\n"),
        );
        dependencies.packs.push(WorkshopDependencyPack {
            id: "extra".into(),
            manifest_toml: String::new(),
            files: BTreeMap::from([(
                pack_path.into(),
                format!("uuid = \"{NOBODY}\"\nname = \"Newer\"\n"),
            )]),
            assets: BTreeMap::new(),
        });
        let catalog = catalog(&files(&[(ROGUE_PATH, &rogue())]), &dependencies);
        let choice = catalog
            .choices
            .factions
            .iter()
            .find(|choice| choice.uuid == NOBODY)
            .unwrap();
        assert_eq!(
            (choice.name.as_str(), choice.origin.as_str()),
            ("Newer", "pack:extra")
        );
        let draft = catalog
            .factions
            .iter()
            .find(|faction| faction.path == ROGUE_PATH)
            .unwrap();
        assert_eq!(draft.enemies[1].name.as_deref(), Some("Newer"));
        let warnings: Vec<&WorkshopFinding> = catalog
            .findings
            .iter()
            .filter(|finding| finding.category == "dependency-duplicate-uuid")
            .collect();
        assert_eq!(warnings.len(), 1, "{:?}", catalog.findings);
        assert_eq!(warnings[0].severity, "warning");
        assert_eq!(warnings[0].file, "assets/factions/z.toml");
        assert!(warnings[0].message.contains(base_path) && warnings[0].message.contains(pack_path));
        // A draft that declares the uuid itself carries the error instead.
        let candidate = files(&[
            (ROGUE_PATH, &rogue()),
            (
                "assets/factions/twin.toml",
                &format!("uuid = \"{NOBODY}\"\nname = \"Twin\"\n"),
            ),
        ]);
        let mut beneath = dependencies.base_files.clone();
        beneath.extend(dependencies.packs[0].files.clone());
        let reported = findings(&candidate, &beneath);
        assert!(located(&reported, "dependency-duplicate-uuid").is_empty());
        assert_eq!(
            located(&reported, "faction-duplicate-uuid"),
            vec![("assets/factions/twin.toml".to_owned(), Some(1))]
        );
    }
}

#[test]
fn shipped_content_has_no_definition_findings() {
    let mut candidate = BTreeMap::new();
    for directory in ["assets/factions", "assets/entities", "assets/worlds"] {
        for entry in std::fs::read_dir(directory).unwrap().flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(".toml") || name.ends_with(".rhai") {
                candidate.insert(
                    format!("{directory}/{name}"),
                    std::fs::read_to_string(&path).unwrap(),
                );
            }
        }
    }
    let findings = findings(&candidate, &BTreeMap::new());
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn a_draft_the_tree_cannot_read_is_reported_in_the_catalog_only() {
    let catalog = catalog(
        &files(&[
            (ROGUE_PATH, "uuid = \"not closed\n"),
            (
                "assets/factions/typed.toml",
                "uuid = \"not-a-uuid\"\nname = \"Typed\"\n",
            ),
        ]),
        &base(),
    );
    assert_eq!(
        catalog
            .findings
            .iter()
            .filter(|finding| finding.category == "runtime-source-invalid")
            .map(|finding| finding.file.as_str())
            .collect::<Vec<_>>(),
        vec![ROGUE_PATH, "assets/factions/typed.toml"]
    );
    assert!(catalog
        .findings
        .iter()
        .any(|finding| finding.category == "faction-invalid-uuid"
            && finding.file == "assets/factions/typed.toml"
            && finding.line == Some(1)));
    assert!(findings(
        &files(&[(ROGUE_PATH, "uuid = \"not closed\n")]),
        &BTreeMap::new()
    )
    .is_empty());
}

#[test]
fn new_faction_source_is_the_runtime_type_serialised() {
    let source = new_faction_source("  Rogue ", ROGUE).unwrap();
    let parsed = crate::ai::faction::parse_faction_config(&source).unwrap();
    assert_eq!(parsed.name, "Rogue");
    assert_eq!(parsed.uuid.to_string(), ROGUE);
    assert!(parsed.enemies.is_empty());
    assert!(parsed.display_name.is_none());
    assert!(parsed.compliance.is_none());
    assert!(source.contains("enemies = []"), "{source}");
    assert!(!source.contains("display_name"), "{source}");
    assert!(new_faction_source("   ", ROGUE).is_err());
    assert!(new_faction_source("Rogue", "not-a-uuid").is_err());
}

#[test]
fn value_checks_follow_the_runtime_types() {
    use Segment::{Index, Key};
    let key = |text: &str| Key(text.into());
    assert!(check_value(
        ROGUE_PATH,
        &[key("enemies"), Index(0)],
        &format!("\"{ALLIANCE}\"")
    )
    .is_ok());
    assert!(check_value(ROGUE_PATH, &[key("enemies"), Index(0)], "\"not-a-uuid\"").is_err());
    assert!(check_value(ROGUE_PATH, &[key("enemies")], &format!("[\"{ALLIANCE}\"]")).is_ok());
    assert!(check_value(ROGUE_PATH, &[key("enemies")], "[1]").is_err());
    assert!(check_value(ROGUE_PATH, &[key("uuid")], "\"nope\"").is_err());
    assert!(check_value(ROGUE_PATH, &[key("compliance"), key("hold")], "\"refuse\"").is_ok());
    assert!(check_value(ROGUE_PATH, &[key("compliance"), key("hold")], "\"maybe\"").is_err());
    assert!(check_value(ROGUE_PATH, &[key("compliance"), key("ack_secs")], "4").is_ok());
    assert!(check_value(ROGUE_PATH, &[key("compliance"), key("ack_secs")], "\"4\"").is_err());
    assert!(check_value(ROGUE_PATH, &[key("compliance"), key("other")], "1").is_err());
    assert!(check_value(ROGUE_PATH, &[key("compliance")], "{ hold = \"refuse\" }").is_ok());
    assert!(check_value(ROGUE_PATH, &[key("compliance")], "{ hold = \"maybe\" }").is_err());
    assert!(
        check_value(ROGUE_PATH, &[key("banner")], "1").is_ok(),
        "unknown keys are free"
    );
    let rung = [key("station"), Index(0), key("rating"), Index(1)];
    let rung_name: Vec<Segment> = rung.iter().cloned().chain([key("name")]).collect();
    assert!(check_value(HULL_PATH, &rung_name, "\"Novice\"").is_ok());
    assert!(check_value(HULL_PATH, &rung_name, "\"  \"").is_err());
    assert!(check_value(HULL_PATH, &rung_name, "3").is_err());
    let automated: Vec<Segment> = rung
        .iter()
        .cloned()
        .chain([key("automated_systems"), Index(0)])
        .collect();
    assert!(check_value(HULL_PATH, &automated, "\"red-alert\"").is_ok());
    assert!(check_value(HULL_PATH, &automated, "3").is_err());
    assert!(check_value(
        HULL_PATH,
        &[key("station"), Index(0), key("visiting_rating")],
        "\"Std\""
    )
    .is_ok());
    assert!(check_value(
        HULL_PATH,
        &[key("station"), Index(0), key("visiting_rating")],
        "1"
    )
    .is_err());
    let rating_path = [key("station"), Index(0), key("rating")];
    assert!(check_table_fields(
        HULL_PATH,
        &rating_path,
        &[
            ("name".into(), "\"Novice\"".into()),
            ("automated_systems".into(), "[]".into())
        ]
    )
    .is_ok());
    assert!(check_table_fields(
        HULL_PATH,
        &rating_path,
        &[("automated_systems".into(), "[]".into())]
    )
    .is_err());
    assert!(
        check_table_fields(HULL_PATH, &rating_path, &[("name".into(), "\"\"".into())]).is_err()
    );
}
