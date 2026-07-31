//! Asset budgets (issue #868).
//!
//! Extracted from what is on disk and what the entity templates declare —
//! never from a running game — so the same checkout always produces the same
//! capture. This is the "find oversized assets before release" story, and for
//! a browser game the shipped byte count is the budget that actually hurts.
//!
//! What is measured, and why it is only this:
//!
//! - `assets.glb.bytes` — one sample per `.glb`. The distribution is the
//!   point: `max` finds the one model that dominates a download.
//! - `assets.glb.total.bytes` — one sample. What a cold visitor pays if
//!   everything loads.
//! - `assets.lod.levels` — one sample per entity template whose model's rig
//!   sidecar declares a `[[lod]]` chain, so a drop in LOD coverage is visible.
//! - `assets.glb.without_lod` — one sample: entity templates naming a `.glb`
//!   with no LOD ladder at all.
//!
//! The ladder moved from the entity's `[[mesh.lod]]` into the model's rig
//! sidecar (issue #914), so coverage is now a *join*: the entity names a model,
//! the model's sidecar says how many levels it has. Both metrics stay keyed by
//! entity template, because "which of my templates has no ladder" is the
//! question the budget is asked.
//!
//! **Triangle and texture counts are deliberately absent.** Both live inside
//! the GLB binary, and reading them means parsing glTF — a real dependency and
//! a real chance of disagreeing with what Bevy actually uploads. Bytes and LOD
//! coverage are honest today; the mesh interior is a separate piece of work
//! that should be built against Bevy's own loader rather than a second
//! opinion about the same file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use vellum_perf::{Capture, Profile, Recorder, Unit};

pub const GLB_BYTES_METRIC: &str = "assets.glb.bytes";
pub const GLB_TOTAL_METRIC: &str = "assets.glb.total.bytes";
pub const LOD_LEVELS_METRIC: &str = "assets.lod.levels";
pub const WITHOUT_LOD_METRIC: &str = "assets.glb.without_lod";

/// The scenario asset captures are filed under.
pub const SCENARIO: &str = "assets";
/// The runtime an asset capture records: no runtime ran at all.
pub const RUNTIME: &str = "static-inventory";

/// What the inventory found, before it becomes a capture.
///
/// Kept as data so the extraction is testable against a fixture directory
/// without going near `vellum-perf` or the real `assets/` tree.
#[derive(Debug, Default, PartialEq)]
pub struct Inventory {
    /// `.glb` path (as declared, forward-slashed) → size in bytes.
    pub glb_bytes: BTreeMap<String, u64>,
    /// Entity template stem → number of declared LOD levels.
    pub lod_levels: BTreeMap<String, u64>,
    /// Entity templates naming a `.glb` with no LOD ladder.
    pub without_lod: Vec<String>,
}

#[derive(Debug)]
pub enum InventoryError {
    Io(String, std::io::Error),
}

impl std::fmt::Display for InventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InventoryError::Io(path, e) => write!(f, "could not read {path:?}: {e}"),
        }
    }
}

/// Walk the model and entity directories under `root`.
///
/// Missing directories are an error rather than an empty inventory: a capture
/// that silently measures nothing would read as "every asset shrank to zero",
/// which is the most alarming possible way to report a wrong path.
pub fn inventory(root: &Path) -> Result<Inventory, InventoryError> {
    let mut found = Inventory::default();

    let models = root.join("assets/models");
    for path in read_dir_sorted(&models)? {
        if path.extension().and_then(|e| e.to_str()) != Some("glb") {
            continue;
        }
        let bytes = std::fs::metadata(&path)
            .map_err(|e| InventoryError::Io(path.display().to_string(), e))?
            .len();
        found.glb_bytes.insert(file_key(&path), bytes);
    }

    let entities = root.join("assets/entities");
    for path in read_dir_sorted(&entities)? {
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| InventoryError::Io(path.display().to_string(), e))?;
        let Some(sidecar) = mesh_sidecar(&text) else {
            continue;
        };
        // A sidecar that is absent or unreadable counts as "no ladder", the
        // same thing the renderer concludes from it.
        let levels = std::fs::read_to_string(root.join(&sidecar))
            .ok()
            .map(|s| sidecar_lod_levels(&s))
            .unwrap_or(0);
        let key = file_key(&path);
        if levels > 0 {
            found.lod_levels.insert(key, levels);
        } else {
            found.without_lod.push(key);
        }
    }

    Ok(found)
}

/// The rig-sidecar path for one entity template's `[mesh]`, or `None` when the
/// template declares no mesh or no `.glb` model (a purely procedural entity has
/// no ladder to be missing).
///
/// Parsed as TOML rather than grepped so a `model` key in an unrelated table
/// cannot be miscounted.
fn mesh_sidecar(text: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(text).ok()?;
    let mesh = value.get("mesh")?;
    let model = mesh
        .get("model")
        .and_then(|m| m.as_str())
        .filter(|m| m.ends_with(".glb"))?;
    Some(crate::model_rig::sidecar_path(
        model,
        mesh.get("variant").and_then(|v| v.as_str()),
    ))
}

/// How many `[[lod]]` levels a model rig sidecar declares.
///
/// Parsed as TOML rather than grepped so a commented-out `[[lod]]` cannot be
/// miscounted as coverage.
fn sidecar_lod_levels(text: &str) -> u64 {
    toml::from_str::<toml::Value>(text)
        .ok()
        .as_ref()
        .and_then(|v| v.get("lod"))
        .and_then(|l| l.as_array())
        .map(|l| l.len() as u64)
        .unwrap_or(0)
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>, InventoryError> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| InventoryError::Io(dir.display().to_string(), e))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    // Sorted so the sample order — and therefore the capture bytes — is the
    // same on every filesystem.
    paths.sort();
    Ok(paths)
}

fn file_key(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Turn an inventory into a capture.
pub fn capture(found: &Inventory, profile: Profile) -> Capture {
    let mut recorder = Recorder::new();
    let mut total = 0u64;
    for bytes in found.glb_bytes.values() {
        recorder.sample(GLB_BYTES_METRIC, Unit::Bytes, *bytes as f64);
        total += bytes;
    }
    recorder.sample(GLB_TOTAL_METRIC, Unit::Bytes, total as f64);
    for levels in found.lod_levels.values() {
        recorder.sample(LOD_LEVELS_METRIC, Unit::Count, *levels as f64);
    }
    recorder.sample(
        WITHOUT_LOD_METRIC,
        Unit::Count,
        found.without_lod.len() as f64,
    );
    recorder.finish(SCENARIO, profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::profile;

    /// The join the budget now makes: a template points at a sidecar, and the
    /// sidecar is what carries the ladder (issue #914).
    #[test]
    fn a_template_resolves_the_sidecar_that_carries_its_ladder() {
        let text = "[mesh]\nmodel = \"assets/models/rock.glb\"\nvariant = \"large\"\n";
        assert_eq!(
            mesh_sidecar(text).as_deref(),
            Some("assets/models/rock.large.toml")
        );
    }

    #[test]
    fn a_template_without_a_variant_resolves_the_default_sidecar() {
        let text = "[mesh]\nmodel = \"assets/models/ship.glb\"\nshape = \"sphere\"\n";
        assert_eq!(
            mesh_sidecar(text).as_deref(),
            Some("assets/models/ship.model.toml")
        );
    }

    #[test]
    fn a_procedural_template_names_no_glb() {
        assert_eq!(
            mesh_sidecar("[mesh]\nshape = \"sphere\"\nradius = 4\n"),
            None
        );
    }

    #[test]
    fn a_template_with_no_mesh_is_skipped_entirely() {
        assert_eq!(mesh_sidecar("name = \"thing\"\n"), None);
    }

    #[test]
    fn a_sidecar_with_a_ladder_reports_its_level_count() {
        let text = r#"
[base]
offset = [0.0, 0.0, 0.0]

[[lod]]
max_distance = 50.0

[[lod]]
max_distance = 100.0
"#;
        assert_eq!(sidecar_lod_levels(text), 2);
    }

    #[test]
    fn a_sidecar_with_no_ladder_reports_zero_levels() {
        assert_eq!(sidecar_lod_levels("[base]\nscale = [1.0, 1.0, 1.0]\n"), 0);
    }

    /// A commented-out ladder is the case grepping would get wrong.
    #[test]
    fn commented_out_lod_is_not_counted() {
        assert_eq!(sidecar_lod_levels("# [[lod]]\n# max_distance = 50.0\n"), 0);
    }

    #[test]
    fn the_capture_totals_the_glb_bytes() {
        let mut found = Inventory::default();
        found.glb_bytes.insert("a.glb".into(), 100);
        found.glb_bytes.insert("b.glb".into(), 400);
        found.lod_levels.insert("rock.toml".into(), 3);
        found.without_lod.push("ship.toml".into());

        let capture = capture(&found, profile(RUNTIME));
        assert_eq!(capture.summaries[GLB_TOTAL_METRIC].summary.max, 500.0);
        assert_eq!(capture.summaries[GLB_BYTES_METRIC].summary.count, 2);
        assert_eq!(capture.summaries[GLB_BYTES_METRIC].summary.max, 400.0);
        assert_eq!(capture.summaries[LOD_LEVELS_METRIC].summary.max, 3.0);
        assert_eq!(capture.summaries[WITHOUT_LOD_METRIC].summary.max, 1.0);
    }

    /// The real tree, because an extractor that works only on fixtures is an
    /// extractor that has never met the assets it budgets.
    #[test]
    fn the_repository_inventory_finds_models_and_ladders() {
        let found = inventory(Path::new(".")).expect("assets directories exist");
        assert!(
            !found.glb_bytes.is_empty(),
            "no .glb files found under assets/models"
        );
        assert!(
            !found.lod_levels.is_empty(),
            "no entity template declares a LOD ladder"
        );
    }

    #[test]
    fn a_missing_tree_is_an_error_not_an_empty_inventory() {
        assert!(inventory(Path::new("no/such/root")).is_err());
    }
}
