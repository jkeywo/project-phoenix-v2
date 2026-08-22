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
//! **Triangle and texture counts are deliberately absent from here.** Both
//! live inside the GLB binary, and reading them off disk would mean parsing
//! glTF — a real dependency and a real chance of disagreeing with what Bevy
//! actually uploads. Bytes and LOD coverage are what a `stat` call can
//! honestly say. The mesh interior is [`mesh`](super::mesh), which reads it
//! through Bevy's own loader rather than as a second opinion about the same
//! file (issue #905).
//!
//! **This scenario gates.** It is the first one to (issue #905): CI runs its
//! report with `--gate`, so a fail exits 3 and turns the run red. It earned
//! that by being machine-independent in fact and not only in principle — the
//! runner's own capture of e87c871 compares at +0.0% drift on every metric.
//! The rule, and why the timing scenarios have not earned it, is in
//! [the module documentation](super).
//!
//! The LOD generator (issue #919, `scripts/generate-lods.mjs`) records the byte
//! size of every file it produces in `scripts/lod-manifest.toml`. It does not
//! measure them a second way: `the_lod_manifest_records_the_bytes_this_inventory_measures`
//! below asserts that what the manifest recorded is what this inventory reads
//! off disk, so there is one byte measurement in the repository with two
//! readers rather than two measurements that can disagree.

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
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".remesh.glb"))
        {
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
    Some(crate::entities::model_rig::sidecar_path(
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

    #[test]
    fn remesh_intermediates_are_not_shipped_glb_inventory() {
        let root = std::env::temp_dir().join(format!(
            "phoenix_perf_assets_remesh_fixture_{}",
            std::process::id()
        ));
        let models = root.join("assets/models");
        let entities = root.join("assets/entities");
        std::fs::create_dir_all(&models).expect("fixture models directory");
        std::fs::create_dir_all(&entities).expect("fixture entities directory");
        std::fs::write(models.join("rock.glb"), [0u8; 7]).expect("runtime model fixture");
        std::fs::write(models.join("rock.remesh.glb"), [0u8; 19])
            .expect("generator intermediate fixture");

        let found = inventory(&root).expect("fixture inventory");
        assert_eq!(found.glb_bytes, BTreeMap::from([("rock.glb".into(), 7)]));

        std::fs::remove_dir_all(&root).ok();
    }

    /// The generated LOD files and this budget agree about their size
    /// (issue #919).
    ///
    /// `scripts/generate-lods.mjs` writes a manifest recording, per generated
    /// `.glb`, the source it came from, the parameters that made it, and how
    /// big it turned out. That last number is *this* module's measurement —
    /// asserted here rather than re-derived, so the pipeline cannot grow a
    /// second opinion about file size. It also means a generated LOD that was
    /// hand-edited, truncated or reverted without regenerating fails `cargo
    /// test`, not only the Node drift check in CI.
    ///
    /// Triangle and texture budgets live in [`super::super::mesh`]; nothing
    /// here counts them.
    #[test]
    fn the_lod_manifest_records_the_bytes_this_inventory_measures() {
        let text = std::fs::read_to_string("scripts/lod-manifest.toml")
            .expect("the LOD manifest is committed alongside the generated files");
        let doc: toml::Value = toml::from_str(&text).expect("the manifest parses as TOML");
        let outputs = doc
            .get("output")
            .and_then(|o| o.as_array())
            .expect("the manifest lists [[output]] records");
        assert!(
            !outputs.is_empty(),
            "the shipped tree generates at least one LOD level"
        );

        let found = inventory(Path::new(".")).expect("assets directories exist");
        let mut problems: Vec<String> = Vec::new();
        for output in outputs {
            let path = output
                .get("path")
                .and_then(|p| p.as_str())
                .expect("every record names its file");
            let recorded = output
                .get("output_bytes")
                .and_then(|b| b.as_integer())
                .expect("every record carries the size it was written at")
                as u64;
            let key = path.rsplit('/').next().unwrap_or(path);
            match found.glb_bytes.get(key) {
                None => problems.push(format!("{path}: recorded in the manifest, missing on disk")),
                Some(&bytes) if bytes != recorded => problems.push(format!(
                    "{path}: manifest recorded {recorded} bytes, disk has {bytes}"
                )),
                Some(_) => {}
            }
        }
        assert!(
            problems.is_empty(),
            "generated LODs have drifted from scripts/lod-manifest.toml — regenerate with \
             `npm run lods` (or re-baseline with `--adopt`) and commit both:\n{}",
            problems.join("\n")
        );
    }
}
