// Base scenario manifest + pre-load scenario/ship catalog (issue #754).
//
// Pure Rust module — no Bevy. Parses the assets-root `scenarios.toml` manifest
// (the authoritative list of selectable root worlds), validates its entries
// with source-located [`WorldFinding`]s, and builds the authoritative
// scenario/ship catalog **before any world is activated**.
//
// The manifest is a thin index: each `[[scenario]]` entry carries an `id`, a
// `world` path, and an optional `label`. Display metadata (title/description)
// and the per-scenario player-ship list are read from the referenced world's
// `[global]` and `[[available_ships]]` sections, so authored data stays
// single-sourced in the world file. This schema is shared with mod-pack
// scenario manifests (issues #759/#760).
//
// World-file I/O stays out of this module: callers pass a `resolve_world`
// closure (path -> Option<world TOML>), mirroring the `WorldSource` pattern in
// `world::validate`. That keeps parse, validation, and catalog-building unit
// testable on native with an in-memory world map, and keeps the wasm/native
// accessors a thin wrapper over the pure core.

use crate::world::config::{parse_world, AvailableShipEntry};
use crate::world::validate::{line_of, Severity, SourceLocation, WorldFinding};

/// One `[[scenario]]` entry in the base scenario manifest.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct ScenarioEntry {
    /// Stable scenario id, unique within the manifest.
    pub id: String,
    /// Path to the selectable root world TOML (`assets/worlds/*.toml`).
    pub world: String,
    /// Optional display label override. When absent, the catalog falls back to
    /// the referenced world's `[global] title`.
    #[serde(default)]
    pub label: Option<String>,
}

/// The parsed base scenario manifest: the ordered list of selectable roots.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Manifest {
    #[serde(default, rename = "scenario")]
    pub scenarios: Vec<ScenarioEntry>,
}

/// Parse the base scenario manifest TOML.
///
/// Returns an `Err` with a human-readable message on TOML syntax errors. Empty
/// or missing `[[scenario]]` tables parse into an empty manifest — semantic
/// problems (no entries, bad references, duplicates) are reported by
/// [`validate_manifest`] as source-located findings, not parse errors.
pub fn parse_manifest(toml_str: &str) -> Result<Manifest, String> {
    toml::from_str(toml_str).map_err(|e| e.to_string())
}

/// Validate a parsed manifest, reporting source-located [`WorldFinding`]s.
///
/// `manifest_toml` is the raw manifest text used for best-effort line lookup.
/// `resolve_world` maps a world path to its TOML content, returning `None` when
/// the world file is missing/unreadable (the caller owns the I/O).
///
/// Error findings (any of which should block using the catalog):
/// * `empty-manifest` — the manifest declares no scenarios.
/// * `invalid-manifest-entry` — an entry with an empty `id` or `world`.
/// * `duplicate-scenario-id` — two entries share an `id`.
/// * `missing-scenario-world` — the referenced world file cannot be resolved.
/// * `unparseable-scenario-world` — the referenced world fails to parse.
pub fn validate_manifest(
    manifest: &Manifest,
    manifest_toml: &str,
    resolve_world: impl Fn(&str) -> Option<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();

    if manifest.scenarios.is_empty() {
        findings.push(finding(
            "empty-manifest",
            manifest_toml,
            "",
            "scenario manifest declares no [[scenario]] entries".to_string(),
        ));
        return findings;
    }

    let mut seen_ids: Vec<&str> = Vec::new();
    for entry in &manifest.scenarios {
        let id = entry.id.trim();
        let world = entry.world.trim();

        if id.is_empty() {
            findings.push(finding(
                "invalid-manifest-entry",
                manifest_toml,
                &entry.world,
                format!("scenario entry (world {:?}) has an empty id", entry.world),
            ));
        }
        if world.is_empty() {
            findings.push(finding(
                "invalid-manifest-entry",
                manifest_toml,
                &entry.id,
                format!("scenario {:?} has an empty world path", entry.id),
            ));
            // Nothing further to check for an entry with no world reference.
            continue;
        }

        if !id.is_empty() {
            if seen_ids.contains(&id) {
                findings.push(finding(
                    "duplicate-scenario-id",
                    manifest_toml,
                    &entry.id,
                    format!("scenario id {:?} is declared more than once", entry.id),
                ));
            } else {
                seen_ids.push(id);
            }
        }

        match resolve_world(world) {
            None => findings.push(finding(
                "missing-scenario-world",
                manifest_toml,
                &entry.world,
                format!(
                    "scenario {:?} references world {:?} which cannot be found",
                    entry.id, entry.world
                ),
            )),
            Some(world_toml) => {
                if let Err(e) = parse_world(&world_toml) {
                    findings.push(finding(
                        "unparseable-scenario-world",
                        manifest_toml,
                        &entry.world,
                        format!(
                            "scenario {:?} world {:?} failed to parse: {e}",
                            entry.id, entry.world
                        ),
                    ));
                }
            }
        }
    }

    findings
}

/// One player-ship option offered by a scenario (reuses the world's
/// [`AvailableShipEntry`]).
pub type CatalogShip = AvailableShipEntry;

/// One selectable scenario in the pre-load catalog, with its display metadata
/// and the ships it offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioCatalogEntry {
    /// Stable scenario id from the manifest.
    pub id: String,
    /// World TOML path.
    pub world: String,
    /// Display label: the manifest entry's `label`, else the world's
    /// `[global] title`, else `None`.
    pub label: Option<String>,
    /// The world's `[global] description`, when present.
    pub description: Option<String>,
    /// The ships this scenario offers — the referenced world's
    /// `[[available_ships]]` list, and *only* those (issue #754 AC4).
    pub ships: Vec<CatalogShip>,
}

/// The authoritative pre-load catalog: the selectable scenarios and their
/// per-scenario ship lists, built from the manifest before any world is active.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenarioCatalog {
    pub scenarios: Vec<ScenarioCatalogEntry>,
}

/// Build the authoritative scenario/ship catalog from a parsed manifest.
///
/// For each manifest entry, resolves and parses the referenced world (cheap
/// data parse — no entity spawn) to read its title/description and
/// `[[available_ships]]`. Entries whose world cannot be resolved or parsed are
/// skipped (they are reported by [`validate_manifest`]); the catalog therefore
/// only ever exposes well-formed selectable scenarios and the ships each one
/// actually offers.
pub fn build_catalog(
    manifest: &Manifest,
    resolve_world: impl Fn(&str) -> Option<String>,
) -> ScenarioCatalog {
    let mut scenarios = Vec::new();
    for entry in &manifest.scenarios {
        if entry.id.trim().is_empty() || entry.world.trim().is_empty() {
            continue;
        }
        let Some(world_toml) = resolve_world(&entry.world) else {
            continue;
        };
        let Ok(world) = parse_world(&world_toml) else {
            continue;
        };
        let label = entry.label.clone().or_else(|| world.global.title.clone());
        scenarios.push(ScenarioCatalogEntry {
            id: entry.id.clone(),
            world: entry.world.clone(),
            label,
            description: world.global.description.clone(),
            ships: world.available_ships.clone(),
        });
    }
    ScenarioCatalog { scenarios }
}

/// Build an error [`WorldFinding`] located in the manifest text.
fn finding(
    category: &'static str,
    manifest_toml: &str,
    reference: &str,
    message: String,
) -> WorldFinding {
    WorldFinding {
        severity: Severity::Error,
        category,
        message,
        source: SourceLocation {
            file: "assets/scenarios.toml".to_string(),
            line: line_of(manifest_toml, reference),
            reference: reference.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const MANIFEST: &str = r#"
[[scenario]]
id = "default"
world = "assets/worlds/default.toml"

[[scenario]]
id = "combat_test"
world = "assets/worlds/combat_test.toml"
"#;

    fn world_with_ships() -> String {
        r#"
[global]
title = "world.default.title"
description = "world.default.description"

[[available_ships]]
template_path = "assets/entities/alliance_cruiser.toml"
label = "Cruiser"

[[available_ships]]
template_path = "assets/entities/alliance_destroyer.toml"
"#
        .to_string()
    }

    fn combat_world() -> String {
        r#"
[global]
title = "world.combat.title"

[[available_ships]]
template_path = "assets/entities/alliance_battleship.toml"
"#
        .to_string()
    }

    fn resolver(map: HashMap<String, String>) -> impl Fn(&str) -> Option<String> {
        move |path: &str| map.get(path).cloned()
    }

    fn full_map() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("assets/worlds/default.toml".to_string(), world_with_ships());
        m.insert("assets/worlds/combat_test.toml".to_string(), combat_world());
        m
    }

    // -- parse ---------------------------------------------------------------

    #[test]
    fn parse_manifest_reads_scenario_entries() {
        let m = parse_manifest(MANIFEST).expect("must parse");
        assert_eq!(m.scenarios.len(), 2);
        assert_eq!(m.scenarios[0].id, "default");
        assert_eq!(m.scenarios[0].world, "assets/worlds/default.toml");
        assert_eq!(m.scenarios[1].id, "combat_test");
    }

    #[test]
    fn parse_manifest_reads_optional_label() {
        let toml = r#"
[[scenario]]
id = "x"
world = "assets/worlds/x.toml"
label = "Custom"
"#;
        let m = parse_manifest(toml).expect("must parse");
        assert_eq!(m.scenarios[0].label.as_deref(), Some("Custom"));
    }

    #[test]
    fn parse_manifest_empty_is_empty_manifest() {
        let m = parse_manifest("").expect("empty parses");
        assert!(m.scenarios.is_empty());
    }

    #[test]
    fn parse_manifest_rejects_toml_syntax_error() {
        assert!(parse_manifest("nope [").is_err());
    }

    // -- validation ----------------------------------------------------------

    #[test]
    fn valid_manifest_produces_no_findings() {
        let m = parse_manifest(MANIFEST).unwrap();
        let findings = validate_manifest(&m, MANIFEST, resolver(full_map()));
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn empty_manifest_is_a_finding() {
        let m = Manifest::default();
        let findings = validate_manifest(&m, "", resolver(full_map()));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "empty-manifest");
        assert!(findings[0].is_error());
    }

    #[test]
    fn missing_world_file_is_source_located_error() {
        let m = parse_manifest(MANIFEST).unwrap();
        // Only default resolves; combat_test is missing.
        let mut map = HashMap::new();
        map.insert("assets/worlds/default.toml".to_string(), world_with_ships());
        let findings = validate_manifest(&m, MANIFEST, resolver(map));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "missing-scenario-world");
        assert_eq!(findings[0].source.file, "assets/scenarios.toml");
        assert_eq!(
            findings[0].source.reference,
            "assets/worlds/combat_test.toml"
        );
        assert!(findings[0].source.line.is_some(), "line should be located");
    }

    #[test]
    fn unparseable_world_is_a_finding() {
        let m = parse_manifest(MANIFEST).unwrap();
        let mut map = full_map();
        map.insert(
            "assets/worlds/combat_test.toml".to_string(),
            "not valid [".to_string(),
        );
        let findings = validate_manifest(&m, MANIFEST, resolver(map));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "unparseable-scenario-world");
    }

    #[test]
    fn duplicate_scenario_id_is_a_finding() {
        let toml = r#"
[[scenario]]
id = "dup"
world = "assets/worlds/default.toml"

[[scenario]]
id = "dup"
world = "assets/worlds/combat_test.toml"
"#;
        let m = parse_manifest(toml).unwrap();
        let findings = validate_manifest(&m, toml, resolver(full_map()));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "duplicate-scenario-id");
    }

    #[test]
    fn empty_id_and_empty_world_are_findings() {
        let toml = r#"
[[scenario]]
id = ""
world = "assets/worlds/default.toml"

[[scenario]]
id = "noworld"
world = ""
"#;
        let m = parse_manifest(toml).unwrap();
        let findings = validate_manifest(&m, toml, resolver(full_map()));
        let cats: Vec<&str> = findings.iter().map(|f| f.category).collect();
        assert!(cats.contains(&"invalid-manifest-entry"));
        assert_eq!(
            cats.iter()
                .filter(|c| **c == "invalid-manifest-entry")
                .count(),
            2
        );
    }

    // -- catalog -------------------------------------------------------------

    #[test]
    fn catalog_exposes_only_scenario_ships() {
        let m = parse_manifest(MANIFEST).unwrap();
        let catalog = build_catalog(&m, resolver(full_map()));
        assert_eq!(catalog.scenarios.len(), 2);

        let default = &catalog.scenarios[0];
        assert_eq!(default.id, "default");
        // Falls back to the world's [global] title.
        assert_eq!(default.label.as_deref(), Some("world.default.title"));
        assert_eq!(
            default.description.as_deref(),
            Some("world.default.description")
        );
        // Only the default world's two ships — not the combat world's.
        assert_eq!(default.ships.len(), 2);
        assert_eq!(
            default.ships[0].template_path,
            "assets/entities/alliance_cruiser.toml"
        );
        assert_eq!(
            default.ships[1].template_path,
            "assets/entities/alliance_destroyer.toml"
        );

        let combat = &catalog.scenarios[1];
        assert_eq!(combat.ships.len(), 1);
        assert_eq!(
            combat.ships[0].template_path,
            "assets/entities/alliance_battleship.toml"
        );
    }

    #[test]
    fn catalog_entry_label_override_wins_over_world_title() {
        let toml = r#"
[[scenario]]
id = "default"
world = "assets/worlds/default.toml"
label = "Override"
"#;
        let m = parse_manifest(toml).unwrap();
        let catalog = build_catalog(&m, resolver(full_map()));
        assert_eq!(catalog.scenarios[0].label.as_deref(), Some("Override"));
    }

    #[test]
    fn catalog_skips_unresolvable_worlds() {
        let m = parse_manifest(MANIFEST).unwrap();
        let mut map = HashMap::new();
        map.insert("assets/worlds/default.toml".to_string(), world_with_ships());
        let catalog = build_catalog(&m, resolver(map));
        // combat_test could not be resolved, so only default is catalogued.
        assert_eq!(catalog.scenarios.len(), 1);
        assert_eq!(catalog.scenarios[0].id, "default");
    }

    #[test]
    fn catalog_scenario_with_no_ships_is_empty_not_missing() {
        let toml = r#"
[[scenario]]
id = "story"
world = "assets/worlds/story.toml"
"#;
        let mut map = HashMap::new();
        map.insert(
            "assets/worlds/story.toml".to_string(),
            "[global]\ntitle = \"world.story.title\"\n".to_string(),
        );
        let m = parse_manifest(toml).unwrap();
        let catalog = build_catalog(&m, resolver(map));
        assert_eq!(catalog.scenarios.len(), 1);
        assert!(catalog.scenarios[0].ships.is_empty());
    }

    // -- shipped manifest ----------------------------------------------------

    /// The real shipped manifest must parse, list exactly the three selectable
    /// roots, and validate cleanly against the shipped world files — the
    /// pre-load catalog is authoritative, so a broken manifest must fail in CI
    /// rather than at host startup.
    #[test]
    fn shipped_manifest_parses_and_validates() {
        let manifest_toml = include_str!("../../assets/scenarios.toml");
        let m = parse_manifest(manifest_toml).expect("scenarios.toml must parse");
        let ids: Vec<&str> = m.scenarios.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["default", "combat_test", "before_the_fire"]);

        let mut map = HashMap::new();
        map.insert(
            "assets/worlds/default.toml".to_string(),
            include_str!("../../assets/worlds/default.toml").to_string(),
        );
        map.insert(
            "assets/worlds/combat_test.toml".to_string(),
            include_str!("../../assets/worlds/combat_test.toml").to_string(),
        );
        map.insert(
            "assets/worlds/before_the_fire.toml".to_string(),
            include_str!("../../assets/worlds/before_the_fire.toml").to_string(),
        );

        let findings = validate_manifest(&m, manifest_toml, resolver(map.clone()));
        assert!(
            findings.is_empty(),
            "shipped manifest must validate cleanly: {findings:?}"
        );

        // The catalog exposes each scenario's own ships, drawn from its world.
        let catalog = build_catalog(&m, resolver(map));
        assert_eq!(catalog.scenarios.len(), 3);
        let default = catalog
            .scenarios
            .iter()
            .find(|s| s.id == "default")
            .unwrap();
        assert_eq!(default.ships.len(), 2);
    }

    // -- exported mod-pack manifest ------------------------------------------

    /// The editor mod-pack exporter (issue #759) writes its `scenarios.toml`
    /// with the SAME `[[scenario]]` schema this module parses — that is the
    /// shared content-pack validation surface the upload path (#760) reuses.
    /// A manifest shaped exactly like the exporter's `buildManifestToml`
    /// output must parse and validate cleanly against the pack's own worlds.
    #[test]
    fn exported_mod_pack_manifest_parses_and_validates() {
        // Byte-for-byte the shape smol-toml emits for the exporter's
        // `{ scenario: [{ id, world, label? }] }` (see editor/mod-pack-export.js
        // buildManifestToml): one entry with a label, one without.
        let manifest_toml = concat!(
            "[[scenario]]\n",
            "id = \"default\"\n",
            "world = \"assets/worlds/default.toml\"\n",
            "label = \"Default\"\n\n",
            "[[scenario]]\n",
            "id = \"skirmish\"\n",
            "world = \"assets/worlds/skirmish.toml\"\n",
        );

        let m = parse_manifest(manifest_toml).expect("exported manifest must parse");
        assert_eq!(m.scenarios.len(), 2);
        assert_eq!(m.scenarios[0].label.as_deref(), Some("Default"));
        assert_eq!(m.scenarios[1].label, None);

        // The pack ships both referenced root worlds — resolve_world reads the
        // pack contents, exactly as an upload would resolve within the archive.
        let mut pack = HashMap::new();
        pack.insert(
            "assets/worlds/default.toml".to_string(),
            "[global]\ntitle = \"world.default.title\"\n".to_string(),
        );
        pack.insert(
            "assets/worlds/skirmish.toml".to_string(),
            "[global]\ntitle = \"world.skirmish.title\"\n".to_string(),
        );

        let findings = validate_manifest(&m, manifest_toml, resolver(pack));
        assert!(
            findings.is_empty(),
            "exported mod-pack manifest must validate cleanly: {findings:?}"
        );
    }

    /// A mod-pack manifest whose root world is absent from the pack must be a
    /// blocking `missing-scenario-world` finding on the same surface — the
    /// editor exporter refuses this case before writing, and the host upload
    /// must reject it too.
    #[test]
    fn exported_manifest_with_unresolved_world_is_rejected() {
        let manifest_toml = concat!(
            "[[scenario]]\n",
            "id = \"ghost\"\n",
            "world = \"assets/worlds/ghost.toml\"\n",
        );
        let m = parse_manifest(manifest_toml).unwrap();
        let findings = validate_manifest(&m, manifest_toml, resolver(HashMap::new()));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "missing-scenario-world");
        assert!(findings[0].is_error());
    }
}
