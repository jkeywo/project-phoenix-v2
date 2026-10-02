//! Native Workshop wrapper around the production `generate-lods.mjs` path.
//! The tool writes only into a private immutable stage until the page reviews
//! and adopts the resulting members as one validated draft transaction.
use super::asset_job::{remove_stage, AssetJob};
use crate::{
    delivery::{http, serve::HostedDocuments},
    entities::model_rig::ModelRig,
    workshop::provider::{Files, Response},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const PREFIX: &str = "/workshop-lod-generation/";
const MANIFEST: &str = "scripts/lod-manifest.toml";

struct ActiveRun {
    id: String,
    job: AssetJob,
    log: PathBuf,
    paths: Vec<String>,
    candidates: BTreeSet<String>,
    required_outputs: BTreeSet<String>,
    sidecar: String,
    source_revision: u64,
}

pub struct LodGeneration {
    documents: HostedDocuments,
    origin: String,
    directory: PathBuf,
    tool_root: PathBuf,
    active: Option<ActiveRun>,
}

impl LodGeneration {
    pub fn new(
        documents: HostedDocuments,
        origin: String,
        tool_root: PathBuf,
        directory: PathBuf,
    ) -> Result<Self, String> {
        remove_stage(&directory);
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        Ok(Self {
            documents,
            origin: origin.trim_end_matches('/').into(),
            directory,
            tool_root,
            active: None,
        })
    }

    #[allow(clippy::disallowed_methods)] // Private run nonce, never simulation identity.
    pub fn start(
        &mut self,
        files: Files,
        sidecar: String,
        source_revision: u64,
        remesh: bool,
    ) -> Result<Response, String> {
        self.retire();
        let source = files
            .get(&sidecar)
            .ok_or("Selected model sidecar is no longer in the draft")?;
        let rig: ModelRig = toml::from_str(
            std::str::from_utf8(source).map_err(|_| "Selected model sidecar is not UTF-8")?,
        )
        .map_err(|error| format!("Selected model sidecar is invalid: {error}"))?;
        let first_glb = rig.lod.iter().find_map(|level| level.model.as_deref());
        let mut candidates = BTreeSet::from([sidecar.clone(), MANIFEST.into()]);
        let mut required_outputs = BTreeSet::new();
        let mut generated = 0;
        for level in &rig.lod {
            let Some(generate) = &level.generate else {
                continue;
            };
            let output = level
                .model
                .as_deref()
                .ok_or("A selected [lod.generate] level names no output GLB")?;
            if !model_glb(output) {
                return Err("Generated LOD output must be a runtime model GLB".into());
            }
            candidates.insert(output.into());
            required_outputs.insert(output.into());
            generated += 1;
            if remesh {
                if let (Some(voxel), Some(source)) = (
                    generate.remesh_voxel_size,
                    generate.source.as_deref().or(first_glb),
                ) {
                    if voxel > 0.0 && generation_source_glb(source) {
                        candidates.insert(source.replace(".glb", ".remesh.glb"));
                    }
                }
            }
        }
        if generated == 0 {
            return Err("Selected model sidecar declares no generated LOD levels".into());
        }

        let id = uuid::Uuid::new_v4().to_string();
        let stage = self.directory.join(&id);
        let mut job = AssetJob::stage(stage.clone(), self.documents.clone(), &files)?;
        let log = stage.join("lod-generation.log");
        let temporary = stage.join("tmp");
        fs::create_dir_all(&temporary).map_err(|error| error.to_string())?;
        let stderr = fs::File::create(&log).map_err(|error| error.to_string())?;
        let stdout = stderr.try_clone().map_err(|error| error.to_string())?;
        let script = self.tool_root.join("scripts/generate-lods.mjs");
        let mut command = Command::new("node");
        command.current_dir(&stage).arg(script).arg(&sidecar);
        if remesh {
            command.arg("--remesh");
        }

        command
            .env("PHOENIX_LOD_TOOL_ROOT", &self.tool_root)
            .env("PHOENIX_LOD_SIDECAR", &sidecar)
            .env("TMP", &temporary)
            .env("TEMP", &temporary)
            .env("TMPDIR", &temporary)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        job.launch(&mut command)
            .map_err(|error| format!("LOD generation tool is unavailable: {error}"))?;
        self.active = Some(ActiveRun {
            id,
            job,
            log,
            paths: Vec::new(),
            candidates,
            required_outputs,
            sidecar,
            source_revision,
        });
        Ok(self.response("running"))
    }

    pub fn status(&mut self) -> Result<Response, String> {
        let Some(active) = self.active.as_mut() else {
            return Err("No LOD generation is active".into());
        };
        if active.job.has_review() {
            return Ok(self.response("ready"));
        }
        let output = match active.job.poll() {
            Ok(Some(output)) => output,
            Ok(None) => return Ok(self.response("running")),
            Err(error) => {
                let message = error.to_string();
                self.retire();
                return Err(message);
            }
        };
        let status = output.status;
        if !status.success() {
            let detail = progress(&active.log).join(" ");
            self.retire();
            return Err(if detail.is_empty() {
                format!("LOD generation failed ({status})")
            } else {
                detail
            });
        }
        let base = format!("{PREFIX}{}/", active.id);
        let mut published = Vec::new();
        for path in &active.candidates {
            let file = active
                .job
                .directory()
                .join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if !file.is_file() {
                continue;
            }
            let bytes = match fs::read(&file) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let message = error.to_string();
                    self.retire();
                    return Err(message);
                }
            };
            let route = format!("{base}{}", published.len());
            active
                .job
                .publish(route, bytes, http::content_type_for(path));
            published.push(path.clone());
        }
        if !complete_review(&active.required_outputs, &published) {
            self.retire();
            return Err("LOD generation did not produce its selected GLB and manifest".into());
        }
        active.paths = published;
        Ok(self.response("ready"))
    }

    fn response(&self, state: &str) -> Response {
        let active = self
            .active
            .as_ref()
            .expect("LOD response requires active run");
        Response::LodGeneration {
            run: active.id.clone(),
            state: state.into(),
            progress: progress(&active.log),
            sidecar: active.sidecar.clone(),
            source_revision: active.source_revision,
            base_url: (active.job.has_review())
                .then(|| format!("{}{PREFIX}{}/", self.origin, active.id)),
            paths: active.paths.clone(),
            required_paths: active.required_outputs.iter().cloned().collect(),
        }
    }

    pub fn cancel(&mut self) -> Response {
        self.retire();
        Response::Done
    }
    pub fn retire(&mut self) {
        self.active.take();
    }
}

fn model_glb(path: &str) -> bool {
    path.starts_with("assets/models/") && path.ends_with(".glb") && !path.contains("..")
}
fn generation_source_glb(path: &str) -> bool {
    (path.starts_with("assets/models/") || path.starts_with("scripts/art/lod-sources/"))
        && path.ends_with(".glb")
        && !path.contains("..")
}
fn complete_review(required: &BTreeSet<String>, published: &[String]) -> bool {
    published.iter().any(|path| path == MANIFEST)
        && required.iter().all(|path| published.contains(path))
}
fn progress(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(64)
        .map(|line| line.chars().take(240).collect())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}
impl Drop for LodGeneration {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[path = "lod_generation_tests.rs"]
mod tests;
