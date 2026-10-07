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

pub use phoenix_content::manifest::*;

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

pub use phoenix_sim_contracts::catalogue::{CatalogShip, ScenarioCatalog, ScenarioCatalogEntry};
