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

use crate::world::config::{parse_world, AvailableShipEntry, ShipSlotConfig};
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
                            .effective_ship_slots()
                            .iter()
                            .flat_map(|slot| slot.ships.iter())
                            .any(|ship| &ship.template_path == ship_path);
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
                    if !parsed.ship_slots.is_empty() && !entry.ships.is_empty() {
                        for slot in &parsed.ship_slots {
                            let retained: Vec<_> = slot
                                .ships
                                .iter()
                                .filter(|ship| {
                                    entry.ships.iter().any(|path| path == &ship.template_path)
                                })
                                .collect();
                            if retained.is_empty() {
                                findings.push(finding(
                                    "empty-curated-ship-slot",
                                    manifest_toml,
                                    &entry.id,
                                    format!(
                                        "scenario {:?} curation leaves ship slot {:?} with no playable hull",
                                        entry.id, slot.id
                                    ),
                                ));
                            } else if !retained
                                .iter()
                                .any(|ship| ship.template_path == slot.default_ship)
                            {
                                findings.push(finding(
                                    "excluded-curated-slot-default",
                                    manifest_toml,
                                    &entry.id,
                                    format!(
                                        "scenario {:?} curation excludes default hull {:?} from ship slot {:?}",
                                        entry.id, slot.default_ship, slot.id
                                    ),
                                ));
                            }
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
    /// The ships this scenario offers — `[[available_ships]]` for a legacy
    /// world, or the deduplicated hull options of its explicit ship slots.
    pub ships: Vec<CatalogShip>,
    /// Authored mission slots after legacy one-slot compatibility synthesis.
    pub slots: Vec<ShipSlotConfig>,
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
        let all_slots = world.effective_ship_slots();
        let available: Vec<_> = if world.ship_slots.is_empty() {
            world.available_ships.clone()
        } else {
            all_slots
                .iter()
                .flat_map(|slot| slot.ships.iter().cloned())
                .fold(Vec::new(), |mut unique, ship| {
                    if !unique
                        .iter()
                        .any(|row: &AvailableShipEntry| row.template_path == ship.template_path)
                    {
                        unique.push(ship);
                    }
                    unique
                })
        };
        let ships = if entry.ships.is_empty() {
            available
        } else {
            available
                .iter()
                .filter(|s| entry.ships.iter().any(|p| p == &s.template_path))
                .cloned()
                .collect()
        };
        let slots = if world.ship_slots.is_empty() {
            all_slots
                .into_iter()
                .map(|mut slot| {
                    if !entry.ships.is_empty() {
                        slot.ships.retain(|ship| {
                            entry.ships.iter().any(|path| path == &ship.template_path)
                        });
                    }
                    slot
                })
                .collect()
        } else {
            let Ok(slots) = crate::ship_slots::curate_ship_slots(&all_slots, &entry.ships) else {
                // The validator reports the exact slot/default failure. A
                // catalogue is selection authority, so it must not publish a
                // scenario whose frozen launch definition violates curation.
                continue;
            };
            slots
        };
        scenarios.push(ScenarioCatalogEntry {
            id: entry.id.clone(),
            world: entry.world.clone(),
            label,
            description: world.global.description.clone(),
            ships,
            slots,
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
#[path = "manifest_tests.rs"]
mod tests;
