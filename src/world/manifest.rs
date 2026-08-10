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
    /// Optional playable-hull curation (issue #917): `template_path` values
    /// this entry restricts the world's `[[available_ships]]` to. Empty (the
    /// default) means "every ship the world offers" — pre-#917 behaviour, and
    /// what every mod-pack manifest exported by #759 still produces. A
    /// non-empty list NEVER edits the referenced world TOML; it filters the
    /// catalog built from it, in the world's own authored order.
    #[serde(default)]
    pub ships: Vec<String>,
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

// ── Mod-pack identity + content compatibility (issue #986) ────────────────────

/// The highest `[pack] format` version this host understands. A pack declaring
/// a format above this is rejected outright, before any of its content is
/// validated (see `world::mod_pack::validate_mod_pack`), so a future format can
/// never bury its one real incompatibility under a wall of content findings.
///
/// This is a code constant on purpose — it is a versioning boundary, not a
/// gameplay value a designer would tune, so it is exempt from the "no hardcoded
/// gameplay values" rule (AGENTS.md).
pub const SUPPORTED_PACK_FORMAT: u32 = 1;

/// The host's declared content identity — the `[content]` block of the base
/// `assets/scenarios.toml`. It is the host side of the mod-pack compatibility
/// contract: a pack's `[pack.requires]` names the `id`/`epoch` it was authored
/// against, and the upload is rejected unless both match.
///
/// This type is *injected* into `validate_mod_pack` rather than read from a host
/// default there, the same discipline the `resolve_base` seam follows: the pure
/// validator keeps no host dependency of its own, and the wasm bridge reads this
/// from `config_cache::get_scenario_manifest_toml()` and hands it in.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct ContentIdentity {
    /// Stable identifier for the shipped content set (e.g. `"phoenix-base"`).
    pub id: String,
    /// Monotonic content revision. A pack authored against an older epoch is
    /// rejected, so incompatible content can never be silently merged.
    pub epoch: i64,
}

/// Read the `[content]` identity block from a base scenario manifest, or `None`
/// when the manifest declares none.
///
/// Deliberately does NOT touch [`parse_manifest`] or the [`Manifest`] struct —
/// the base manifest still parses through the unchanged reader with no knowledge
/// of `[content]`; this is a separate, additive read used only by the mod-pack
/// upload seam.
pub fn parse_content_identity(manifest_toml: &str) -> Option<ContentIdentity> {
    #[derive(serde::Deserialize)]
    struct Doc {
        content: Option<ContentIdentity>,
    }
    toml::from_str::<Doc>(manifest_toml)
        .ok()
        .and_then(|d| d.content)
}

/// The `[pack.requires]` compatibility clause: the base content identity a pack
/// was authored against. Both fields are optional at the type level so a pack
/// that omits them parses (and is then rejected as a mismatch), rather than
/// failing to parse with a confusing TOML error.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct PackRequires {
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub content_epoch: Option<i64>,
}

/// The required top-level `[pack]` identity table a mod pack's `scenarios.toml`
/// carries (issue #986). `format` is mandatory — it is the version discriminator
/// the compatibility gate reads first; the display/identity fields default to
/// empty so an otherwise-parseable pack surfaces a semantic finding
/// (`invalid-pack-id`) rather than a parse error.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct PackMeta {
    /// Manifest format version. A pack whose `format` exceeds
    /// [`SUPPORTED_PACK_FORMAT`] is rejected before any content validation.
    pub format: u32,
    /// Stable pack id.
    #[serde(default)]
    pub id: String,
    /// Human-facing version string (e.g. `"1.0.0"`).
    #[serde(default)]
    pub version: String,
    /// Display name shown in the host status when the pack is accepted.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Base content this pack requires to be compatible.
    #[serde(default)]
    pub requires: PackRequires,
}

/// A parsed mod-pack manifest: its `[pack]` identity header (absent when the
/// required table is missing — reported as `missing-pack-header` downstream)
/// plus the `[[scenario]]` [`Manifest`] the base reader already understands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackManifest {
    pub pack: Option<PackMeta>,
    pub manifest: Manifest,
}

/// Parse a mod-pack `scenarios.toml`: the `[pack]` identity header on top of the
/// shared `[[scenario]]` schema.
///
/// Wraps [`parse_manifest`] for the scenario list (so a pack manifest and the
/// base manifest validate on exactly the same surface) and additionally reads
/// the top-level `[pack]` table. The base `assets/scenarios.toml`, which has no
/// `[pack]` block, still parses here with `pack: None`.
pub fn parse_pack_manifest(toml_str: &str) -> Result<PackManifest, String> {
    #[derive(serde::Deserialize)]
    struct PackDoc {
        pack: Option<PackMeta>,
    }
    let manifest = parse_manifest(toml_str)?;
    let doc: PackDoc = toml::from_str(toml_str).map_err(|e| e.to_string())?;
    Ok(PackManifest {
        pack: doc.pack,
        manifest,
    })
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
            Some(world_toml) => match parse_world(&world_toml) {
                Err(e) => {
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
                Ok(parsed) => {
                    // Curated hull list (issue #917): every listed template_path
                    // must be one the world actually offers, or the manifest is
                    // curating a ship that can never appear.
                    for ship_path in &entry.ships {
                        let offered = parsed
                            .available_ships
                            .iter()
                            .any(|s| &s.template_path == ship_path);
                        if !offered {
                            findings.push(finding(
                                "unknown-scenario-ship",
                                manifest_toml,
                                ship_path,
                                format!(
                                    "scenario {:?} curates ship {:?} which world {:?} does not offer",
                                    entry.id, ship_path, entry.world
                                ),
                            ));
                        }
                    }
                }
            },
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
    /// Provenance: the pack id this scenario came from, or `None` for a
    /// base-manifest scenario (issue #987). `build_catalog` always sets `None`;
    /// [`build_merged_catalog`] stamps each mod scenario with its pack id.
    pub origin: Option<String>,
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
        // Curated hull list (issue #917): a non-empty `entry.ships` restricts
        // the catalog to those template paths, in the WORLD's authored order
        // — the manifest curates, it never reorders. An empty list (the
        // default) keeps every ship the world offers, unchanged from
        // pre-#917 behaviour.
        let ships = if entry.ships.is_empty() {
            world.available_ships.clone()
        } else {
            world
                .available_ships
                .iter()
                .filter(|s| entry.ships.iter().any(|p| p == &s.template_path))
                .cloned()
                .collect()
        };
        scenarios.push(ScenarioCatalogEntry {
            id: entry.id.clone(),
            world: entry.world.clone(),
            label,
            description: world.global.description.clone(),
            ships,
            origin: None,
        });
    }
    ScenarioCatalog { scenarios }
}

/// The merged base+mod scenario catalog plus any warnings raised while merging
/// (issue #987). Kept as one return value so the caller surfaces both together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergedCatalog {
    pub catalog: ScenarioCatalog,
    /// Non-blocking findings (currently `duplicate-scenario-id` warnings for
    /// cross-pack id collisions resolved by load order).
    pub findings: Vec<WorldFinding>,
}

/// Build the merged scenario catalog from the base manifest PLUS an ORDERED
/// slice of validated mod-pack manifests (issue #760 AC3, issue #987).
///
/// `mods` is `(pack_id, manifest)` pairs in LOAD ORDER (oldest → newest); every
/// manifest — base and each mod — resolves through the same overlay-aware
/// `resolve_world` closure (the caller consults the winning pack in the overlay
/// stack first, then base content), so a mod scenario's root world is read from
/// the WINNING pack for that path — which can differ from the manifest-listing
/// pack when a later pack overrides the same world path. The merged catalog contains regular scenarios and
/// manifest-listed mod scenarios ONLY — a world present in the overlay but not
/// named by any manifest never appears as a selectable scenario.
///
/// Duplicate scenario ids resolve by LOAD ORDER: a later entry REPLACES an
/// earlier one of the same id. Replacing a BASE scenario is the sanctioned mod
/// override and stays silent (behaviour unchanged from #760). Replacing a
/// scenario that came from an EARLIER PACK is a cross-pack collision and raises a
/// non-blocking `duplicate-scenario-id` warning naming both packs. Each mod
/// scenario is stamped with its pack id in [`ScenarioCatalogEntry::origin`].
pub fn build_merged_catalog(
    base: &Manifest,
    mods: &[(&str, &Manifest)],
    resolve_world: impl Fn(&str) -> Option<String>,
) -> MergedCatalog {
    let mut catalog = build_catalog(base, &resolve_world);
    let mut findings = Vec::new();
    for (pack_id, modm) in mods {
        let mod_catalog = build_catalog(modm, &resolve_world);
        for mut entry in mod_catalog.scenarios {
            entry.origin = Some((*pack_id).to_string());
            if let Some(existing) = catalog.scenarios.iter_mut().find(|s| s.id == entry.id) {
                if let Some(prev) = existing.origin.clone() {
                    findings.push(WorldFinding {
                        severity: Severity::Warning,
                        category: "duplicate-scenario-id",
                        message: format!(
                            "scenario id {:?} from pack {:?} overrides the same id from pack {:?} (load order wins)",
                            entry.id, pack_id, prev
                        ),
                        source: SourceLocation {
                            file: "assets/scenarios.toml".to_string(),
                            line: None,
                            reference: entry.id.clone(),
                        },
                    });
                }
                *existing = entry;
            } else {
                catalog.scenarios.push(entry);
            }
        }
    }
    MergedCatalog { catalog, findings }
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
    fn parse_manifest_ships_defaults_to_empty() {
        let m = parse_manifest(MANIFEST).expect("must parse");
        assert!(m.scenarios[0].ships.is_empty());
    }

    #[test]
    fn parse_manifest_reads_optional_ships() {
        let toml = r#"
[[scenario]]
id = "x"
world = "assets/worlds/x.toml"
ships = ["assets/entities/alliance_destroyer.toml"]
"#;
        let m = parse_manifest(toml).expect("must parse");
        assert_eq!(
            m.scenarios[0].ships,
            vec!["assets/entities/alliance_destroyer.toml".to_string()]
        );
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
    fn curated_ship_offered_by_world_produces_no_finding() {
        let toml = r#"
[[scenario]]
id = "combat_test"
world = "assets/worlds/combat_test.toml"
ships = ["assets/entities/alliance_battleship.toml"]
"#;
        let m = parse_manifest(toml).unwrap();
        let findings = validate_manifest(&m, toml, resolver(full_map()));
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn curated_ship_not_offered_by_world_is_a_finding() {
        let toml = r#"
[[scenario]]
id = "combat_test"
world = "assets/worlds/combat_test.toml"
ships = ["assets/entities/alliance_destroyer.toml"]
"#;
        // combat_world() only offers the battleship — the destroyer is not one
        // of its [[available_ships]], so curating it is a source-located error.
        let m = parse_manifest(toml).unwrap();
        let findings = validate_manifest(&m, toml, resolver(full_map()));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "unknown-scenario-ship");
        assert_eq!(
            findings[0].source.reference,
            "assets/entities/alliance_destroyer.toml"
        );
        assert!(findings[0].is_error());
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
    fn catalog_curates_ships_to_the_manifest_allowlist() {
        // world_with_ships() offers cruiser then destroyer; curate down to just
        // the destroyer without touching the world file at all (issue #917).
        let toml = r#"
[[scenario]]
id = "default"
world = "assets/worlds/default.toml"
ships = ["assets/entities/alliance_destroyer.toml"]
"#;
        let m = parse_manifest(toml).unwrap();
        let catalog = build_catalog(&m, resolver(full_map()));
        assert_eq!(catalog.scenarios.len(), 1);
        assert_eq!(catalog.scenarios[0].ships.len(), 1);
        assert_eq!(
            catalog.scenarios[0].ships[0].template_path,
            "assets/entities/alliance_destroyer.toml"
        );
    }

    #[test]
    fn catalog_ship_curation_preserves_world_authored_order() {
        // Curation lists the ships out of order; the catalog keeps the WORLD's
        // order, not the manifest's — the manifest only filters membership.
        let toml = r#"
[[scenario]]
id = "default"
world = "assets/worlds/default.toml"
ships = ["assets/entities/alliance_destroyer.toml", "assets/entities/alliance_cruiser.toml"]
"#;
        let m = parse_manifest(toml).unwrap();
        let catalog = build_catalog(&m, resolver(full_map()));
        assert_eq!(catalog.scenarios[0].ships.len(), 2);
        // world_with_ships() authors cruiser first, then destroyer.
        assert_eq!(
            catalog.scenarios[0].ships[0].template_path,
            "assets/entities/alliance_cruiser.toml"
        );
        assert_eq!(
            catalog.scenarios[0].ships[1].template_path,
            "assets/entities/alliance_destroyer.toml"
        );
    }

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

    // -- merged catalog (issue #760, AC3) ------------------------------------

    #[test]
    fn merged_catalog_contains_only_manifest_listed_scenarios() {
        let base = parse_manifest(MANIFEST).unwrap();
        let mod_manifest = parse_manifest(
            r#"
[[scenario]]
id = "mod_skirmish"
world = "assets/worlds/mod_skirmish.toml"
"#,
        )
        .unwrap();

        // Overlay holds the base worlds, the listed mod world, AND an extra
        // mod world that no manifest names — the latter must NOT appear.
        let mut map = full_map();
        map.insert(
            "assets/worlds/mod_skirmish.toml".to_string(),
            "[global]\ntitle = \"world.mod_skirmish.title\"\n".to_string(),
        );
        map.insert(
            "assets/worlds/unlisted_mod.toml".to_string(),
            "[global]\ntitle = \"world.unlisted.title\"\n".to_string(),
        );

        let merged = build_merged_catalog(&base, &[("modpack", &mod_manifest)], resolver(map));
        let ids: Vec<&str> = merged
            .catalog
            .scenarios
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, ["default", "combat_test", "mod_skirmish"]);
        assert!(
            !ids.contains(&"unlisted_mod"),
            "an overlay world not named by a manifest must not be selectable"
        );
        // The appended mod scenario is stamped with its pack id (issue #987).
        let skirmish = merged
            .catalog
            .scenarios
            .iter()
            .find(|s| s.id == "mod_skirmish")
            .unwrap();
        assert_eq!(skirmish.origin.as_deref(), Some("modpack"));
    }

    #[test]
    fn merged_catalog_mod_entry_replaces_base_id() {
        let base = parse_manifest(MANIFEST).unwrap();
        let mod_manifest = parse_manifest(
            r#"
[[scenario]]
id = "default"
world = "assets/worlds/mod_default.toml"
label = "Modded Default"
"#,
        )
        .unwrap();
        let mut map = full_map();
        map.insert(
            "assets/worlds/mod_default.toml".to_string(),
            "[global]\ntitle = \"world.mod_default.title\"\n".to_string(),
        );
        let merged = build_merged_catalog(&base, &[("modpack", &mod_manifest)], resolver(map));
        // Still two ids (default replaced in place, not duplicated).
        assert_eq!(merged.catalog.scenarios.len(), 2);
        let default = merged
            .catalog
            .scenarios
            .iter()
            .find(|s| s.id == "default")
            .unwrap();
        assert_eq!(default.world, "assets/worlds/mod_default.toml");
        assert_eq!(default.label.as_deref(), Some("Modded Default"));
        // Replacing a BASE scenario is the sanctioned override — no warning.
        assert!(
            merged.findings.is_empty(),
            "base-vs-mod replacement must not warn: {:?}",
            merged.findings
        );
    }

    #[test]
    fn merged_catalog_without_mods_is_base_only() {
        let base = parse_manifest(MANIFEST).unwrap();
        let merged = build_merged_catalog(&base, &[], resolver(full_map()));
        assert_eq!(merged.catalog.scenarios.len(), 2);
        assert!(merged.findings.is_empty());
    }

    #[test]
    fn merged_catalog_duplicate_scenario_id_across_packs_resolves_by_load_order() {
        let base = parse_manifest(MANIFEST).unwrap();
        let mod_a = parse_manifest(
            "[[scenario]]\nid = \"shared\"\nworld = \"assets/worlds/shared_a.toml\"\n",
        )
        .unwrap();
        let mod_b = parse_manifest(
            "[[scenario]]\nid = \"shared\"\nworld = \"assets/worlds/shared_b.toml\"\n",
        )
        .unwrap();
        let mut map = full_map();
        map.insert(
            "assets/worlds/shared_a.toml".to_string(),
            "[global]\ntitle = \"world.shared_a.title\"\n".to_string(),
        );
        map.insert(
            "assets/worlds/shared_b.toml".to_string(),
            "[global]\ntitle = \"world.shared_b.title\"\n".to_string(),
        );
        // packA loaded first, packB last → packB wins the shared id.
        let merged = build_merged_catalog(
            &base,
            &[("packA", &mod_a), ("packB", &mod_b)],
            resolver(map),
        );
        let shared = merged
            .catalog
            .scenarios
            .iter()
            .find(|s| s.id == "shared")
            .unwrap();
        assert_eq!(shared.world, "assets/worlds/shared_b.toml");
        assert_eq!(shared.origin.as_deref(), Some("packB"));
        // The cross-pack collision raised a non-blocking warning naming both.
        assert_eq!(merged.findings.len(), 1);
        assert_eq!(merged.findings[0].category, "duplicate-scenario-id");
        assert!(!merged.findings[0].is_error());
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

    /// The demo curation manifest (issue #917): must parse, curate the
    /// catalogue down to exactly `combat_test`, and — without editing
    /// `combat_test.toml`, which still authors four `[[available_ships]]` —
    /// resolve the ship list down to exactly the Alliance Destroyer.
    #[test]
    fn demo_manifest_curates_to_combat_test_and_the_destroyer() {
        let manifest_toml = include_str!("../../assets/scenarios.demo.toml");
        let m = parse_manifest(manifest_toml).expect("scenarios.demo.toml must parse");
        let ids: Vec<&str> = m.scenarios.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["combat_test"]);

        let mut map = HashMap::new();
        map.insert(
            "assets/worlds/combat_test.toml".to_string(),
            include_str!("../../assets/worlds/combat_test.toml").to_string(),
        );

        let findings = validate_manifest(&m, manifest_toml, resolver(map.clone()));
        assert!(
            findings.is_empty(),
            "demo manifest must validate cleanly: {findings:?}"
        );

        let catalog = build_catalog(&m, resolver(map));
        assert_eq!(catalog.scenarios.len(), 1);
        assert_eq!(catalog.scenarios[0].id, "combat_test");
        assert_eq!(catalog.scenarios[0].ships.len(), 1);
        assert_eq!(
            catalog.scenarios[0].ships[0].template_path,
            "assets/entities/alliance_destroyer.toml"
        );

        // combat_test.toml itself is untouched: it still authors all four
        // hulls. Curation happens only in the manifest's ships allowlist.
        let combat_toml = include_str!("../../assets/worlds/combat_test.toml");
        let world = parse_world(combat_toml).expect("combat_test.toml must parse");
        assert_eq!(world.available_ships.len(), 4);
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

    // -- pack manifest + content identity (issue #986) -----------------------

    const PACK_MANIFEST: &str = r#"
[pack]
format = 1
id = "aurora-skirmish"
version = "1.0.0"
name = "Aurora Skirmish"
author = "Fixture Author"
description = "A deterministic fixture pack."

[pack.requires]
content_id = "phoenix-base"
content_epoch = 1

[[scenario]]
id = "aurora_skirmish"
world = "assets/worlds/aurora_skirmish.toml"
"#;

    #[test]
    fn parse_pack_manifest_reads_header_and_scenarios() {
        let pm = parse_pack_manifest(PACK_MANIFEST).expect("must parse");
        let pack = pm.pack.expect("has a [pack] header");
        assert_eq!(pack.format, 1);
        assert_eq!(pack.id, "aurora-skirmish");
        assert_eq!(pack.version, "1.0.0");
        assert_eq!(pack.name, "Aurora Skirmish");
        assert_eq!(pack.author.as_deref(), Some("Fixture Author"));
        assert_eq!(pack.requires.content_id.as_deref(), Some("phoenix-base"));
        assert_eq!(pack.requires.content_epoch, Some(1));
        // The [[scenario]] half is exactly what the base reader produces.
        assert_eq!(pm.manifest.scenarios.len(), 1);
        assert_eq!(pm.manifest.scenarios[0].id, "aurora_skirmish");
    }

    #[test]
    fn parse_pack_manifest_optional_fields_default() {
        // A minimal header: only format + id, no version/name/author/requires.
        let toml = "[pack]\nformat = 1\nid = \"x\"\n\n[[scenario]]\nid = \"s\"\nworld = \"assets/worlds/s.toml\"\n";
        let pm = parse_pack_manifest(toml).expect("must parse");
        let pack = pm.pack.expect("has a [pack] header");
        assert_eq!(pack.version, "");
        assert_eq!(pack.name, "");
        assert_eq!(pack.author, None);
        assert_eq!(pack.requires, PackRequires::default());
    }

    #[test]
    fn parse_pack_manifest_without_pack_header_is_none() {
        // The base manifest shape (no [pack]) still parses through the pack
        // reader, with an absent header — the missing-pack-header seam.
        let pm = parse_pack_manifest(MANIFEST).expect("must parse");
        assert!(pm.pack.is_none());
        assert_eq!(pm.manifest.scenarios.len(), 2);
    }

    #[test]
    fn base_manifest_still_parses_via_unchanged_parse_manifest() {
        // A manifest carrying [pack] + [content] must remain readable by the
        // untouched base reader, which ignores both and sees only [[scenario]].
        let m = parse_manifest(PACK_MANIFEST).expect("base reader ignores [pack]");
        assert_eq!(m.scenarios.len(), 1);
        assert_eq!(m.scenarios[0].id, "aurora_skirmish");
    }

    #[test]
    fn parse_content_identity_reads_content_block() {
        let toml = "[content]\nid = \"phoenix-base\"\nepoch = 3\n\n[[scenario]]\nid = \"s\"\nworld = \"w\"\n";
        let id = parse_content_identity(toml).expect("reads [content]");
        assert_eq!(id.id, "phoenix-base");
        assert_eq!(id.epoch, 3);
    }

    #[test]
    fn parse_content_identity_absent_is_none() {
        assert!(parse_content_identity(MANIFEST).is_none());
    }

    #[test]
    fn shipped_base_manifest_declares_content_identity() {
        // The real shipped manifest must carry the host side of the mod-pack
        // compatibility contract, so an upload always has a base to match.
        let id = parse_content_identity(include_str!("../../assets/scenarios.toml"))
            .expect("assets/scenarios.toml must declare [content]");
        assert!(!id.id.trim().is_empty(), "content id must not be empty");
    }

    #[test]
    fn shipped_demo_manifest_declares_content_identity() {
        let id = parse_content_identity(include_str!("../../assets/scenarios.demo.toml"))
            .expect("assets/scenarios.demo.toml must declare [content]");
        assert!(!id.id.trim().is_empty(), "content id must not be empty");
    }
}
