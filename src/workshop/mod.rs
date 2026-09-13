//! Offline Workshop services over explicit source bundles. These functions do
//! not install content, start a simulation, or touch a running session.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::entities::config::EntityConfig;
use crate::entities::config_cache::ActivePack;
use crate::entities::include_resolve::{canonical_template_path, resolve_template};
use crate::entities::loader::TemplateLoader;
use crate::world::load::{load, LoadPolicy, LoadRequest, WorldReader};
use crate::world::manifest::{parse_content_identity, parse_manifest};
use crate::world::mod_pack::{read_store_zip, validate_mod_pack};
use crate::world::script::load::ScriptResolver;
use crate::world::validate::{
    validate_composition_with_fragments, Severity, WorldFinding, WorldSource,
};

pub mod archive;
pub mod document;
#[cfg(not(target_arch = "wasm32"))]
pub mod provider;
pub mod test_protocol;

#[cfg(target_arch = "wasm32")]
mod wasm;

/// Immutable dependencies, oldest pack first. The editable candidate is passed
/// separately and always wins. No runtime cache or filesystem is consulted.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WorkshopDependencies {
    pub base_files: BTreeMap<String, String>,
    #[serde(default)]
    pub base_assets: BTreeMap<String, Vec<u8>>,
    #[serde(default)]
    pub packs: Vec<WorkshopDependencyPack>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkshopDependencyPack {
    pub id: String,
    pub manifest_toml: String,
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub assets: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkshopFinding {
    pub severity: String,
    pub category: String,
    pub message: String,
    pub file: String,
    pub line: Option<usize>,
}

impl From<WorldFinding> for WorkshopFinding {
    fn from(value: WorldFinding) -> Self {
        Self {
            severity: match value.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            }
            .into(),
            category: value.category.into(),
            message: value.message,
            file: value.source.file,
            line: value.source.line,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct WorkshopValidation {
    pub accepted: bool,
    pub findings: Vec<WorkshopFinding>,
}

impl WorkshopValidation {
    fn error(&mut self, category: &str, file: &str, message: String) {
        self.findings.push(WorkshopFinding {
            severity: "error".into(),
            category: category.into(),
            message,
            file: file.into(),
            line: None,
        });
    }

    fn extend(&mut self, findings: impl IntoIterator<Item = WorldFinding>) {
        for finding in findings {
            let finding = WorkshopFinding::from(finding);
            if !self.findings.contains(&finding) {
                self.findings.push(finding);
            }
        }
    }
}

struct Sources(BTreeMap<String, String>);

impl WorldReader for Sources {
    fn read(&self, path: &str) -> Option<String> {
        self.0.get(&canonical_template_path(path)).cloned()
    }
}

impl ScriptResolver for Sources {
    fn read(&self, path: &str) -> Option<String> {
        WorldReader::read(self, path)
    }
}

impl TemplateLoader for Sources {
    fn load_template(&self, path: &str) -> Option<EntityConfig> {
        resolve_template(path, &self.0).ok()?.parse().ok()
    }
    fn absence_is_final(&self) -> bool {
        true
    }
}

/// Runtime validation of the exact unsaved candidate archive. The ordinary
/// upload validator supplies the manifest/identity/path/include/script gate.
/// The ordinary world loader additionally compiles each selected root and its
/// static children, so script-spawn references use the exact compiled source
/// set, including read-only dependencies. We retain its ledger as unused data.
pub fn validate_pack(bytes: &[u8], dependencies: &WorkshopDependencies) -> WorkshopValidation {
    crate::world::script::init_hashing_seed();
    let mut report = WorkshopValidation::default();
    let Some(identity) = dependencies
        .base_files
        .get("assets/scenarios.toml")
        .and_then(|text| parse_content_identity(text))
    else {
        report.error(
            "missing-base-content",
            "assets/scenarios.toml",
            "The read-only base content manifest is unavailable or has no content identity.".into(),
        );
        return report;
    };
    let active: Vec<ActivePack> = dependencies
        .packs
        .iter()
        .map(|pack| ActivePack {
            id: pack.id.clone(),
            manifest_toml: pack.manifest_toml.clone(),
            files: pack.files.clone().into_iter().collect(),
            ..Default::default()
        })
        .collect();
    let mut beneath = dependencies.base_files.clone();
    for pack in &dependencies.packs {
        beneath.extend(pack.files.clone());
    }
    let base_templates = Sources(beneath.clone());
    let validation = validate_mod_pack(
        bytes,
        &identity,
        |path| {
            dependencies
                .base_files
                .get(&canonical_template_path(path))
                .cloned()
        },
        &base_templates,
        &active,
    );
    report.extend(validation.findings);
    let Ok(candidate) = read_store_zip(bytes) else {
        return report;
    };
    let mut sources = beneath;
    sources.extend(candidate.clone());
    let sources = Sources(sources);

    // Other complete source types parse through their runtime owner. Entity
    // members may be partial include fragments, so the existing pack gate
    // checks their syntax and validates COMPOSED reachable templates instead
    // of requiring every fragment to be a complete EntityConfig on its own.
    for (path, text) in &candidate {
        let result = if path.starts_with("assets/worlds/") && path.ends_with(".toml") {
            crate::world::config::parse_world(text).map(|_| ())
        } else if path.starts_with("assets/factions/") && path.ends_with(".toml") {
            crate::ai::faction::parse_faction_config(text)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else if path.starts_with("assets/models/") && path.ends_with(".toml") {
            crate::entities::model_rig::ModelRig::from_toml(text)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            Ok(())
        };
        if let Err(error) = result {
            report.error("runtime-source-invalid", path, error);
        }
    }
    if let Some(manifest) = candidate
        .get("scenarios.toml")
        .and_then(|text| parse_manifest(text).ok())
    {
        for scenario in manifest.scenarios {
            validate_world(&scenario.world, &sources, &mut report);
        }
    }
    report.accepted = !report
        .findings
        .iter()
        .any(|finding| finding.severity == "error");
    report
}

fn validate_world(path: &str, sources: &Sources, report: &mut WorkshopValidation) {
    let read = |path: &str| load(LoadRequest::new(path, sources, sources, LoadPolicy::Merge));
    let root = match read(path) {
        Ok(root) => root,
        Err(error) => {
            report.error("runtime-world-invalid", path, error.to_string());
            return;
        }
    };
    let mut children = Vec::new();
    for child_path in &root.config.extra_worlds {
        match read(child_path) {
            Ok(child) => children.push((child_path.clone(), child)),
            Err(error) => report.error("runtime-world-invalid", child_path, error.to_string()),
        }
    }
    for world in std::iter::once(&root).chain(children.iter().map(|(_, child)| child)) {
        if let Some(compiled) = &world.scripts {
            report.extend(compiled.findings.clone());
        }
    }
    // Keep the source records separate from the borrowed validation views.
    let root_text = WorldReader::read(sources, path).unwrap_or_default();
    let mut root_source = WorldSource::new(path, &root_text, &root.config);
    if let Some(compiled) = &root.scripts {
        root_source = root_source.with_resolved_script_spawns(&compiled.spawned_templates);
    }
    let child_sources: Vec<_> = children
        .iter()
        .map(|(path, world)| {
            let text = sources
                .0
                .get(&canonical_template_path(path))
                .map(String::as_str)
                .unwrap_or("");
            let mut source = WorldSource::new(path, text, &world.config);
            if let Some(compiled) = &world.scripts {
                source = source.with_resolved_script_spawns(&compiled.spawned_templates);
            }
            source
        })
        .collect();
    report.extend(validate_composition_with_fragments(
        &root_source,
        &child_sources,
        sources,
        &sources.0,
    ));
}

/// A selected project's source set uses the ordinary project manifest and the
/// same offline world/include/script validators. No pack header is invented,
/// and no process-global template cache can fill a missing source member.
pub fn validate_project(files: &BTreeMap<String, Vec<u8>>) -> WorkshopValidation {
    crate::world::script::init_hashing_seed();
    let mut report = WorkshopValidation::default();
    let mut text_files = BTreeMap::new();
    for (path, bytes) in files {
        if !path.ends_with(".toml") && !path.ends_with(".rhai") {
            continue;
        }
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                if path.ends_with(".toml") {
                    if let Err(error) = toml::from_str::<toml::Value>(text) {
                        report.error("runtime-source-invalid", path, error.to_string());
                    }
                }
                text_files.insert(path.clone(), text.to_string());
            }
            Err(error) => report.error("runtime-source-invalid", path, error.to_string()),
        }
    }
    let sources = Sources(text_files);
    if let Some(source) = sources.0.get(crate::sound_cues::PATH) {
        if let Err(error) =
            crate::sound_cues::validate_source(source, |path| files.contains_key(path))
        {
            report.error("invalid-sound-cues", crate::sound_cues::PATH, error);
        }
    }
    report.extend(crate::world::mod_pack::validate_pack_scripts(&sources.0));
    for (path, text) in &sources.0 {
        let result = if path.starts_with("assets/worlds/") && path.ends_with(".toml") {
            crate::world::config::parse_world(text).map(|_| ())
        } else if path.starts_with("assets/factions/") && path.ends_with(".toml") {
            crate::ai::faction::parse_faction_config(text)
                .map(|_| ())
                .map_err(|error| error.to_string())
        } else if path.starts_with("assets/models/") && path.ends_with(".toml") {
            crate::entities::model_rig::ModelRig::from_toml(text)
                .map(|_| ())
                .map_err(|error| error.to_string())
        } else {
            Ok(())
        };
        if let Err(error) = result {
            report.error("runtime-source-invalid", path, error);
        }
    }
    let path = "assets/scenarios.toml";
    match sources.0.get(path).map(|source| parse_manifest(source)) {
        Some(Ok(manifest)) => {
            report.extend(
                crate::world::manifest::validate_manifest(&manifest, &sources.0[path], |path| {
                    sources.0.get(path).cloned()
                })
                .into_iter()
                .map(|mut finding| {
                    if finding.source.file == "scenarios.toml" {
                        finding.source.file = path.into();
                    }
                    finding
                }),
            );
            for scenario in manifest.scenarios {
                validate_world(&scenario.world, &sources, &mut report);
            }
        }
        Some(Err(error)) => report.error("runtime-manifest-invalid", path, error.to_string()),
        None => report.error(
            "runtime-manifest-invalid",
            path,
            "Missing project manifest".into(),
        ),
    }
    report.accepted = !report
        .findings
        .iter()
        .any(|finding| finding.severity == "error");
    report
}

#[cfg(test)]
mod tests;
