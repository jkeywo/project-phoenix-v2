//! Native Workshop wrapper around the production `generate-lods.mjs` path.
//! The tool writes only into a private immutable stage until the page reviews
//! and adopts the resulting members as one validated draft transaction.
use crate::{
    delivery::{http, serve::HostedDocuments},
    entities::model_rig::ModelRig,
    workshop::provider::{Files, Response},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

const PREFIX: &str = "/workshop-lod-generation/";
const MANIFEST: &str = "scripts/lod-manifest.toml";

struct ActiveRun {
    id: String,
    stage: PathBuf,
    child: Option<Child>,
    log: PathBuf,
    routes: Vec<String>,
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
        retire_directory(&directory);
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
        fs::create_dir_all(&stage).map_err(|error| error.to_string())?;
        let started = (|| {
            for (path, bytes) in &files {
                let target = stage.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::write(target, bytes).map_err(|error| error.to_string())?;
            }
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
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            let child = command
                .env("PHOENIX_LOD_TOOL_ROOT", &self.tool_root)
                .env("PHOENIX_LOD_SIDECAR", &sidecar)
                .env("TMP", &temporary)
                .env("TEMP", &temporary)
                .env("TMPDIR", &temporary)
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr))
                .spawn()
                .map_err(|error| format!("LOD generation tool is unavailable: {error}"))?;
            Ok::<_, String>((child, log))
        })();
        let (child, log) = match started {
            Ok(value) => value,
            Err(error) => {
                retire_directory(&stage);
                return Err(error);
            }
        };
        self.active = Some(ActiveRun {
            id,
            stage,
            child: Some(child),
            log,
            routes: Vec::new(),
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
        if !active.routes.is_empty() {
            return Ok(self.response("ready"));
        }
        let Some(child) = active.child.as_mut() else {
            return Err("LOD generation has no process".into());
        };
        let status = match child.try_wait() {
            Ok(Some(status)) => status,
            Ok(None) => return Ok(self.response("running")),
            Err(error) => {
                let message = error.to_string();
                self.retire();
                return Err(message);
            }
        };
        active.child.take();
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
                .stage
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
            self.documents
                .publish_bytes(route.clone(), bytes, http::content_type_for(path), true);
            active.routes.push(route);
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
            base_url: (!active.routes.is_empty())
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
        if let Some(mut active) = self.active.take() {
            if let Some(mut child) = active.child.take() {
                terminate_tree(&mut child);
            }
            for route in active.routes {
                self.documents.withdraw(&route);
            }
            retire_directory(&active.stage);
        }
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
fn retire_directory(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
fn terminate_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        // Node is only the coordinator; gltf-transform or Blender may be its
        // current child. Kill the whole process tree before deleting the stage.
        if let Ok(mut killer) = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if !wait_bounded(&mut killer, std::time::Duration::from_secs(2)) {
                let _ = killer.kill();
            }
        }
    }
    #[cfg(unix)]
    {
        if let Ok(mut killer) = Command::new("kill")
            .args(["-KILL", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if !wait_bounded(&mut killer, std::time::Duration::from_secs(2)) {
                let _ = killer.kill();
            }
        }
    }
    if !wait_bounded(child, std::time::Duration::from_secs(2)) {
        let _ = child.kill();
        wait_bounded(child, std::time::Duration::from_secs(2));
    }
}
fn wait_bounded(child: &mut Child, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Err(_) => return false,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(None) => return false,
        }
    }
}
impl Drop for LodGeneration {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "phoenix-lod-generation-{name}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn progress_is_a_bounded_tail_and_start_refuses_a_sidecar_without_generation() {
        let root = temporary("root");
        let stages = temporary("stages");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&stages).unwrap();
        let log = root.join("progress.log");
        fs::write(
            &log,
            (0..80)
                .map(|index| format!("{index}:{}\n", "x".repeat(300)))
                .collect::<String>(),
        )
        .unwrap();
        let lines = progress(&log);
        assert_eq!(lines.len(), 64);
        assert!(lines.iter().all(|line| line.chars().count() <= 240));
        assert!(lines.first().unwrap().starts_with("16:"));

        let documents = HostedDocuments::default();
        let mut run = LodGeneration::new(
            documents,
            "http://127.0.0.1:7".into(),
            root.clone(),
            stages.clone(),
        )
        .unwrap();
        let refused = run
            .start(
                BTreeMap::from([("assets/models/ship.model.toml".into(), b"[base]\n".to_vec())]),
                "assets/models/ship.model.toml".into(),
                3,
                false,
            )
            .unwrap_err();
        assert!(refused.contains("declares no generated LOD"));
        assert!(stages.read_dir().unwrap().next().is_none());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(stages);
    }

    #[test]
    fn generated_members_stay_in_the_runtime_model_namespace() {
        assert!(model_glb("assets/models/ship_lod1.glb"));
        assert!(!model_glb("assets/models/../private.glb"));
        assert!(!model_glb("scripts/output.glb"));
        assert!(generation_source_glb("scripts/art/lod-sources/ship.glb"));
        assert!(!generation_source_glb(
            "scripts/art/lod-sources/../private.glb"
        ));
        let required = BTreeSet::from(["assets/models/ship_lod1.glb".into()]);
        assert!(!complete_review(
            &required,
            &["assets/models/ship.remesh.glb".into(), MANIFEST.into()]
        ));
    }

    #[test]
    fn cancellation_stops_a_live_descendant_before_the_private_stage_is_removed() {
        let stage = temporary("cancel-tree");
        fs::create_dir_all(&stage).unwrap();
        let marker = stage.join("descendant.txt");
        let descendant =
            "setInterval(()=>require('fs').appendFileSync(process.env.PHOENIX_TEST_MARKER,'x'),20)";
        let parent = format!("require('child_process').spawn(process.execPath,['-e',{}],{{stdio:'ignore',env:process.env}});setInterval(()=>{{}},1000)", serde_json::to_string(descendant).unwrap());
        let mut command = Command::new("node");
        command
            .args(["-e", &parent])
            .env("PHOENIX_TEST_MARKER", &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(250));
        terminate_tree(&mut child);
        let before = fs::metadata(&marker).map(|value| value.len()).unwrap_or(0);
        assert!(
            before > 0,
            "the live descendant never reached its staged output"
        );
        std::thread::sleep(std::time::Duration::from_millis(150));
        let after = fs::metadata(&marker).map(|value| value.len()).unwrap_or(0);
        assert_eq!(
            before, after,
            "the generator descendant survived cancellation"
        );
        retire_directory(&stage);
        assert!(!stage.exists());
    }
}
