//! Selected native Authoring roots. The private host creates this capability;
//! no request can select, expand or replace its filesystem authority.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{document, WorkshopDependencies, WorkshopValidation};
pub mod assets;
pub mod preview_snapshot;
pub mod test_snapshot;

pub type Files = BTreeMap<String, Vec<u8>>;
const MAX_BYTES: usize = 512 * 1024 * 1024;
const MAX_FILES: usize = 16_384;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceKind {
    Project,
    Mod,
}

#[derive(Debug)]
pub struct WorkshopRequest {
    pub id: u64,
    pub operation: Operation,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Operation {
    ShipSchema,
    Load,
    LoadSources,
    LoadDependencies,
    ValidateSources {
        files: assets::Sources,
    },
    SaveSources {
        files: assets::Sources,
        expected_revision: String,
    },
    AssetRead {
        reference: assets::AssetReference,
        offset: usize,
    },
    AssetBegin {
        length: usize,
    },
    AssetChunk {
        token: String,
        offset: usize,
        bytes: Vec<u8>,
    },
    AssetFinish {
        token: String,
    },
    AssetCancel {
        token: String,
    },
    Validate {
        files: Files,
    },
    Save {
        files: Files,
        expected_revision: String,
    },
    Inspect {
        source: String,
        document_path: String,
    },
    Patch {
        source: String,
        patch: document::Patch,
    },
    ScriptHostFunctions,
    ScriptDiagnostics {
        source: String,
        line_offset: usize,
    },
    Definitions {
        files: BTreeMap<String, String>,
    },
    Edit {
        source: String,
        edit: document::EditRequest,
    },
    NewFaction {
        name: String,
        uuid: String,
    },
    Composition {
        files: BTreeMap<String, String>,
    },
    Compose {
        files: BTreeMap<String, String>,
        request: super::composition::ComposeRequest,
    },
    NewWorld {
        title: String,
    },
    Entity {
        files: BTreeMap<String, String>,
        path: String,
    },
    EntityEdit {
        files: BTreeMap<String, String>,
        request: super::entity::EntityEditRequest,
    },
    EntityMaterialise {
        files: BTreeMap<String, String>,
        path: String,
        address: String,
    },
    Presets {
        files: BTreeMap<String, String>,
        path: String,
    },
    PresetsEdit {
        files: BTreeMap<String, String>,
        request: super::presets::PresetEditRequest,
    },
    NewPreset {
        /// Spelled `preset_id` because the request ENVELOPE owns the key `id`
        /// (`codec::decode_workshop_request` removes it before the operation is
        /// deserialized), so a preset's own id cannot travel under that name.
        preset_id: String,
        label: String,
    },
    RecoveryLoad,
    RecoverySave {
        record: String,
        expected_revision: String,
    },
    RecoveryClear,
    TestStart {
        files: assets::Sources,
        selection: test_snapshot::TestSelection,
        #[serde(default)]
        breakpoint: Option<crate::workshop::test_protocol::TestBreakpoint>,
    },
    TestCatalog {
        files: BTreeMap<String, String>,
    },
    TestControl {
        control: crate::workshop::test_protocol::TestControl,
    },
    TestStatus,
    TestStop,
    PreviewStart {
        files: assets::Sources,
        selection: crate::workshop::test_protocol::PreviewSelection,
    },
    PreviewRelease {
        capture: String,
    },
    PreviewStop,
    BillboardCaptureStart {
        files: assets::Sources,
        sidecar: String,
        lod: usize,
        source_revision: u64,
    },
    BillboardCaptureStatus,
    BillboardCaptureCancel,
    LodGenerateStart {
        files: assets::Sources,
        sidecar: String,
        source_revision: u64,
        remesh: bool,
    },
    LodGenerateStatus,
    LodGenerateCancel,
}

#[derive(Debug, Serialize)]
pub struct WorkshopResponse {
    pub id: u64,
    #[serde(flatten)]
    pub result: Response,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Response {
    ShipSchema {
        schema: super::WorkshopShipSchema,
    },
    Sources {
        kind: WorkspaceKind,
        revision: String,
        files: assets::Sources,
    },
    Dependencies {
        base_files: BTreeMap<String, String>,
        packs: Vec<DependencySources>,
    },
    AssetChunk {
        bytes: Vec<u8>,
    },
    AssetUpload {
        token: String,
    },
    AssetStored {
        reference: assets::AssetReference,
    },
    Loaded {
        kind: WorkspaceKind,
        revision: String,
        files: Files,
    },
    Validated {
        report: WorkshopValidation,
    },
    Saved {
        revision: String,
    },
    Fields {
        fields: Vec<document::Field>,
    },
    Patched {
        source: String,
    },
    ScriptHostFunctions {
        functions: Vec<crate::world::script::authoring::HostFn>,
    },
    ScriptDiagnostics {
        diagnostics: Vec<crate::world::script::authoring::ScriptDiagnostic>,
    },
    // Boxed like `Test`: a whole catalog would otherwise make every refusal
    // carry its size.
    Definitions {
        catalog: Box<super::definitions::DefinitionCatalog>,
    },
    Composition {
        catalog: Box<super::composition::CompositionCatalog>,
    },
    Entity {
        composition: Box<super::entity::EntityComposition>,
    },
    Presets {
        presets: Box<super::presets::PresetCatalog>,
    },
    Recovery {
        recovery: Option<RecoveryRecord>,
    },
    Done,
    Test {
        run: Option<Box<crate::workshop::test_protocol::TestStatus>>,
    },
    TestCatalog {
        catalog: test_snapshot::TestCatalog,
    },
    Preview {
        capture: String,
        base_url: String,
        paths: Vec<String>,
        revision: String,
        selection: crate::workshop::test_protocol::PreviewSelection,
    },
    BillboardCapture {
        capture: String,
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        image_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sidecar: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lod: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_revision: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        yaw_views: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolution: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pitch: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        paths: Vec<String>,
    },
    LodGeneration {
        run: String,
        state: String,
        progress: Vec<String>,
        sidecar: String,
        source_revision: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        paths: Vec<String>,
        required_paths: Vec<String>,
    },
    Refused {
        message: String,
        report: Option<WorkshopValidation>,
    },
}

#[derive(Debug, Serialize)]
pub struct DependencySources {
    id: String,
    manifest_toml: String,
    files: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRecord {
    pub revision: String,
    pub record: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Transaction {
    root: String,
    before: BTreeMap<String, Option<Vec<u8>>>,
    after: BTreeMap<String, Option<Vec<u8>>>,
}

/// Every operation is synchronous and serialized by the private surface owner.
/// Holds the OS claim for this selected root until the capability is dropped.
pub struct NativeWorkshopProvider {
    root: PathBuf,
    private: PathBuf,
    kind: WorkspaceKind,
    dependencies: WorkshopDependencies,
    baseline: Files,
    revision: String,
    assets: assets::AssetStore,
    test_support: Files,
    _claim: File,
}

impl NativeWorkshopProvider {
    pub(crate) fn prepare_billboard_capture(
        &mut self,
        sources: assets::Sources,
    ) -> Result<Files, Response> {
        self.prepare_native_model_tool(
            sources,
            "Native billboard capture requires a selected project root",
        )
    }

    pub(crate) fn prepare_lod_generation(
        &mut self,
        sources: assets::Sources,
    ) -> Result<Files, Response> {
        self.prepare_native_model_tool(
            sources,
            "Native LOD generation requires a selected project root",
        )
    }

    fn prepare_native_model_tool(
        &mut self,
        sources: assets::Sources,
        refusal: &str,
    ) -> Result<Files, Response> {
        if self.kind != WorkspaceKind::Project {
            return Err(Response::Refused {
                message: refusal.into(),
                report: None,
            });
        }
        let files = self
            .assets
            .materialize(sources)
            .map_err(|message| Response::Refused {
                message,
                report: None,
            })?;
        self.check_files(&files)
            .map_err(|message| Response::Refused {
                message,
                report: None,
            })?;
        Ok(files)
    }

    #[cfg(feature = "server")]
    pub(crate) fn capture_paths(&self) -> (PathBuf, PathBuf) {
        (self.root.clone(), self.private.join("billboard-captures"))
    }

    #[cfg(feature = "server")]
    pub(crate) fn lod_generation_paths(&self) -> (PathBuf, PathBuf) {
        (self.root.clone(), self.private.join("lod-generation"))
    }
    /// Host-only document lifecycle boundary. Completed immutable versions and
    /// accepted writes survive; an abandoned upload cannot block the next view.
    pub(crate) fn retire_view(&mut self) {
        self.assets.retire_view();
    }
    #[cfg(feature = "server")]
    pub(crate) fn test_directory(&self) -> PathBuf {
        self.private.join("test-runs")
    }
    pub fn open(
        kind: WorkspaceKind,
        root: impl AsRef<Path>,
        recovery_dir: impl AsRef<Path>,
        dependencies: WorkshopDependencies,
    ) -> Result<Self, String> {
        let root = fs::canonicalize(root).map_err(io_error)?;
        if !root.is_dir() {
            return Err("Selected Workshop root is not a directory".into());
        }
        let key = format!(
            "{:016x}",
            vellum_digest::fnv1a(root.to_string_lossy().as_bytes())
        );
        let private = recovery_dir.as_ref().join(key);
        fs::create_dir_all(&private).map_err(io_error)?;
        let claim = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(private.join("workspace.lock"))
            .map_err(io_error)?;
        claim
            .try_lock()
            .map_err(|_| "This Workshop root is already open".to_string())?;
        let mut provider = Self {
            assets: assets::AssetStore::new(private.join("assets")),
            test_support: Files::new(),
            root,
            private,
            kind,
            dependencies,
            baseline: Files::new(),
            revision: String::new(),
            _claim: claim,
        };
        provider.finish_transaction()?;
        provider.baseline = provider.read_files()?;
        provider.test_support = provider.capture_test_support()?;
        provider.revision = revision(&provider.baseline);
        Ok(provider)
    }

    /// The host's embedded bridge is the only caller. A delivered browser page
    /// has no endpoint for this function and can never create the capability.
    pub fn handle_json(&mut self, json: &str) -> String {
        let response = if json.len() > MAX_BYTES.saturating_mul(4) {
            WorkshopResponse {
                id: 0,
                result: refused("Workshop request is too large"),
            }
        } else {
            match crate::core::codec::decode_workshop_request(json) {
                Ok(request) => self.handle(request),
                Err(message) => WorkshopResponse {
                    id: 0,
                    result: refused(message),
                },
            }
        };
        crate::core::codec::encode_workshop_response(&response)
            .expect("Workshop responses contain only finite source data")
    }

    pub fn handle(&mut self, request: WorkshopRequest) -> WorkshopResponse {
        let result = self.apply(request.operation).unwrap_or_else(refused);
        WorkshopResponse {
            id: request.id,
            result,
        }
    }

    fn apply(&mut self, operation: Operation) -> Result<Response, String> {
        Ok(match operation {
            Operation::ShipSchema => Response::ShipSchema {
                schema: super::ship_schema(),
            },
            Operation::LoadSources => Response::Sources {
                kind: self.kind,
                revision: self.revision.clone(),
                files: self.assets.compact(&self.baseline)?,
            },
            Operation::LoadDependencies => Response::Dependencies {
                base_files: self.dependencies.base_files.clone(),
                packs: self
                    .dependencies
                    .packs
                    .iter()
                    .map(|pack| DependencySources {
                        id: pack.id.clone(),
                        manifest_toml: pack.manifest_toml.clone(),
                        files: pack.files.clone(),
                    })
                    .collect(),
            },
            Operation::ValidateSources { files } => {
                let files = self.assets.materialize(files)?;
                return self.apply(Operation::Validate { files });
            }
            Operation::SaveSources {
                files,
                expected_revision,
            } => {
                let files = self.assets.materialize(files)?;
                return self.apply(Operation::Save {
                    files,
                    expected_revision,
                });
            }
            Operation::AssetRead { reference, offset } => Response::AssetChunk {
                bytes: self.assets.read_chunk(reference, offset)?,
            },
            Operation::AssetBegin { length } => Response::AssetUpload {
                token: self.assets.begin(length)?,
            },
            Operation::AssetChunk {
                token,
                offset,
                bytes,
            } => {
                self.assets.append(&token, offset, &bytes)?;
                Response::Done
            }
            Operation::AssetFinish { token } => Response::AssetStored {
                reference: self.assets.finish(&token)?,
            },
            Operation::AssetCancel { token } => {
                self.assets.cancel(&token)?;
                Response::Done
            }
            Operation::Load => Response::Loaded {
                kind: self.kind,
                revision: self.revision.clone(),
                files: self.baseline.clone(),
            },
            Operation::Validate { files } => {
                self.check_files(&files)?;
                Response::Validated {
                    report: self.validate(&files),
                }
            }
            Operation::Save {
                files,
                expected_revision,
            } => {
                self.check_files(&files)?;
                if expected_revision != self.revision || self.read_files()? != self.baseline {
                    return Err("Workshop source changed on disk. Reopen it before saving; the draft is retained.".into());
                }
                let report = self.validate(&files);
                if !report.accepted {
                    return Ok(Response::Refused {
                        message: "Runtime validation refused the save".into(),
                        report: Some(report),
                    });
                }
                let after: BTreeMap<_, _> = files
                    .keys()
                    .chain(self.baseline.keys())
                    .filter(|path| self.baseline.get(*path) != files.get(*path))
                    .map(|path| (path.clone(), files.get(path).cloned()))
                    .collect();
                if !after.is_empty() {
                    let before = after
                        .keys()
                        .map(|path| (path.clone(), self.baseline.get(path).cloned()))
                        .collect();
                    let transaction = Transaction {
                        root: self.root.to_string_lossy().into_owned(),
                        before,
                        after,
                    };
                    let text = crate::core::codec::encode_workshop_transaction(&transaction)?;
                    atomic_write(&self.private.join("transaction.json"), text.as_bytes())?;
                    // The durable intent precedes every replacement. A crash or
                    // IO failure resumes this exact validated save at next open.
                    self.finish_transaction()?;
                }
                self.baseline = files;
                self.revision = revision(&self.baseline);
                Response::Saved {
                    revision: self.revision.clone(),
                }
            }
            Operation::Inspect {
                source,
                document_path,
            } => Response::Fields {
                fields: document::fields(&source, &document_path)?,
            },
            Operation::Patch { source, patch } => Response::Patched {
                source: document::patch(&source, &patch)?,
            },
            Operation::ScriptHostFunctions => Response::ScriptHostFunctions {
                functions: crate::world::script::authoring::host_fns().to_vec(),
            },
            Operation::ScriptDiagnostics {
                source,
                line_offset,
            } => Response::ScriptDiagnostics {
                diagnostics: crate::world::script::authoring::script_diagnostics(
                    &source,
                    line_offset,
                ),
            },
            // The same read-only bundle Validate resolves against: a mod's
            // definitions see the base set and the packs beneath it, while a
            // project IS the whole content set and nothing lies beneath it —
            // a faction the project deletes must dangle in the panel exactly
            // as Check reports it.
            Operation::Definitions { files } => Response::Definitions {
                catalog: Box::new(super::definitions::catalog(
                    &files,
                    self.reference_dependencies(),
                )),
            },
            Operation::Edit { source, edit } => Response::Patched {
                source: document::edit(&source, &edit)?,
            },
            Operation::NewFaction { name, uuid } => Response::Patched {
                source: super::definitions::new_faction_source(&name, &uuid)?,
            },
            // Composition resolves against the same bundle Definitions does:
            // a project is the whole content set, so a child it does not
            // carry is missing exactly as Check reports it (issue #1475).
            Operation::Composition { files } => Response::Composition {
                catalog: Box::new(super::composition::catalog(
                    &files,
                    self.reference_dependencies(),
                )),
            },
            Operation::Compose { files, request } => Response::Patched {
                source: super::composition::compose(
                    &files,
                    self.reference_dependencies(),
                    &request,
                )?,
            },
            Operation::NewWorld { title } => Response::Patched {
                source: super::composition::new_world_source(&title)?,
            },
            // Entity composition resolves against the same bundle the other
            // two catalogs do, and for the same reason: a project IS the whole
            // content set, so a fragment it does not carry is missing exactly
            // as Check reports it (issue #1476).
            Operation::Entity { files, path } => Response::Entity {
                composition: Box::new(super::entity::catalog(
                    &files,
                    self.reference_dependencies(),
                    &path,
                )),
            },
            Operation::EntityEdit { files, request } => Response::Patched {
                source: super::entity::compose(&files, self.reference_dependencies(), &request)?,
            },
            Operation::EntityMaterialise {
                files,
                path,
                address,
            } => Response::Patched {
                source: super::entity::materialise(
                    &files,
                    self.reference_dependencies(),
                    &path,
                    &address,
                )?,
            },
            // Role presets resolve against the same bundle the other catalogs
            // do: a widget's `ship` may name an entity a DEPENDENCY's world
            // declares, so the reference set is candidate ∪ dependencies
            // exactly as Check resolves it (issue #1477).
            Operation::Presets { files, path } => Response::Presets {
                presets: Box::new(super::presets::catalog(
                    &files,
                    self.reference_dependencies(),
                    &path,
                )),
            },
            Operation::PresetsEdit { files, request } => Response::Patched {
                source: super::presets::compose(&files, self.reference_dependencies(), &request)?,
            },
            Operation::NewPreset { preset_id, label } => Response::Patched {
                source: super::presets::new_preset_source(&preset_id, &label)?,
            },
            Operation::RecoveryLoad => {
                let recovery = read_optional(&self.private.join("draft.json"))?
                    .map(|bytes| crate::core::codec::decode_workshop_recovery(&bytes))
                    .transpose()?;
                Response::Recovery { recovery }
            }
            Operation::RecoverySave {
                record,
                expected_revision,
            } => {
                if record.len() > MAX_BYTES {
                    return Err("Workshop recovery record is too large".into());
                }
                let recovery = RecoveryRecord {
                    record,
                    revision: expected_revision,
                };
                atomic_write(
                    &self.private.join("draft.json"),
                    crate::core::codec::encode_workshop_recovery(&recovery)?.as_bytes(),
                )?;
                Response::Done
            }
            Operation::RecoveryClear => {
                remove_optional(&self.private.join("draft.json"))?;
                Response::Done
            }
            Operation::TestCatalog { files } => Response::TestCatalog {
                catalog: self.test_catalog(files)?,
            },
            Operation::TestStart { .. }
            | Operation::TestControl { .. }
            | Operation::TestStatus
            | Operation::TestStop
            | Operation::PreviewStart { .. }
            | Operation::PreviewRelease { .. }
            | Operation::PreviewStop
            | Operation::BillboardCaptureStart { .. }
            | Operation::BillboardCaptureStatus
            | Operation::BillboardCaptureCancel
            | Operation::LodGenerateStart { .. }
            | Operation::LodGenerateStatus
            | Operation::LodGenerateCancel => {
                return Err(
                    "Disposable Test and preview require their explicit offline native shell"
                        .into(),
                );
            }
        })
    }

    /// The read-only bundle a catalog resolves references against: a mod's
    /// definitions and composition see the base set and the packs beneath it,
    /// while a project IS the whole content set and nothing lies beneath it.
    fn reference_dependencies(&self) -> &WorkshopDependencies {
        static NOTHING: WorkshopDependencies = WorkshopDependencies {
            base_files: BTreeMap::new(),
            base_assets: BTreeMap::new(),
            packs: Vec::new(),
        };
        match self.kind {
            WorkspaceKind::Project => &NOTHING,
            WorkspaceKind::Mod => &self.dependencies,
        }
    }

    fn validate(&self, files: &Files) -> WorkshopValidation {
        match self.kind {
            WorkspaceKind::Project => super::validate_project(files),
            WorkspaceKind::Mod => match super::archive::store_zip(files) {
                Ok(bytes) => super::validate_pack(&bytes, &self.dependencies),
                Err(message) => {
                    let mut report = WorkshopValidation::default();
                    report.error("archive-invalid", "scenarios.toml", message);
                    report
                }
            },
        }
    }

    fn check_files(&self, files: &Files) -> Result<(), String> {
        if files.len() > MAX_FILES || files.values().map(Vec::len).sum::<usize>() > MAX_BYTES {
            return Err("Workshop source bundle is too large".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for path in files.keys() {
            if !allowed_path(self.kind, path) {
                return Err(format!("Unsupported Workshop source path: {path}"));
            }
            if !names.insert(path.to_ascii_lowercase()) {
                return Err(format!("Duplicate Workshop source path: {path}"));
            }
            if self.resolve(path)?.is_dir() {
                return Err(format!("Workshop source path names a directory: {path}"));
            }
        }
        Ok(())
    }

    fn read_files(&self) -> Result<Files, String> {
        let mut files = Files::new();
        self.walk(&self.root, &mut files)?;
        if self.kind == WorkspaceKind::Project {
            for manifest in [
                "scripts/lod-capture-manifest.toml",
                "scripts/lod-manifest.toml",
            ] {
                let path = self
                    .root
                    .join(manifest.replace('/', std::path::MAIN_SEPARATOR_STR));
                match fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                        return Err(format!(
                            "Linked Workshop paths are not supported: {manifest}"
                        ));
                    }
                    Ok(_) => {
                        files.insert(manifest.into(), fs::read(path).map_err(io_error)?);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(io_error(error)),
                }
            }
        }
        self.check_files(&files)?;
        Ok(files)
    }

    fn walk(&self, directory: &Path, files: &mut Files) -> Result<(), String> {
        for entry in fs::read_dir(directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            let relative = path
                .strip_prefix(&self.root)
                .map_err(io_error)?
                .to_string_lossy()
                .replace('\\', "/");
            // The project root is explicit authority over authored content,
            // never over code, git metadata or endpoint-private files.
            if entry.file_type().map_err(io_error)?.is_dir() {
                if relative == "assets"
                    || relative.starts_with("assets/")
                    || matches!(
                        relative.as_str(),
                        "scripts" | "scripts/art" | "scripts/art/lod-sources"
                    )
                    || relative.starts_with("scripts/art/lod-sources/")
                {
                    self.resolve(&relative)?;
                    self.walk(&path, files)?;
                }
            } else if allowed_path(self.kind, &relative) {
                self.resolve(&relative)?;
                let size = entry.metadata().map_err(io_error)?.len();
                if size > MAX_BYTES as u64 {
                    return Err("Workshop source member is too large".into());
                }
                files.insert(relative, fs::read(path).map_err(io_error)?);
                if files.len() > MAX_FILES
                    || files.values().map(Vec::len).sum::<usize>() > MAX_BYTES
                {
                    return Err("Workshop source bundle is too large".into());
                }
            } else if entry.file_type().map_err(io_error)?.is_symlink()
                && (relative.starts_with("assets")
                    || relative.starts_with("scripts/art/lod-sources"))
            {
                return Err(format!(
                    "Linked Workshop paths are not supported: {relative}"
                ));
            }
        }
        Ok(())
    }

    fn resolve(&self, relative: &str) -> Result<PathBuf, String> {
        if !safe_path(relative) {
            return Err(format!("Invalid Workshop path: {relative}"));
        }
        let mut path = self.root.clone();
        for component in relative.split('/') {
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink()
                        || !fs::canonicalize(&path)
                            .map_err(io_error)?
                            .starts_with(&self.root)
                    {
                        return Err(format!(
                            "Linked Workshop paths are not supported: {relative}"
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(path)
    }

    fn finish_transaction(&mut self) -> Result<(), String> {
        let path = self.private.join("transaction.json");
        let Some(bytes) = read_optional(&path)? else {
            return Ok(());
        };
        let transaction = crate::core::codec::decode_workshop_transaction(&bytes)?;
        if transaction.root != self.root.to_string_lossy()
            || transaction.before.len() != transaction.after.len()
        {
            return Err("Invalid Workshop save recovery record".into());
        }
        self.check_files(
            &transaction
                .after
                .iter()
                .map(|(name, value)| (name.clone(), value.clone().unwrap_or_default()))
                .collect(),
        )?;
        // Check the whole transaction before touching any file, including when
        // resuming a save which replaced only a prefix before a hard shutdown.
        for (name, after) in &transaction.after {
            let before = transaction
                .before
                .get(name)
                .ok_or("Invalid Workshop save recovery record")?;
            let current = read_optional(&self.resolve(name)?)?;
            if &current != after && &current != before {
                return Err(format!(
                    "Workshop save recovery conflicts with an external edit: {name}"
                ));
            }
        }
        for (name, after) in &transaction.after {
            let target = self.resolve(name)?;
            if &read_optional(&target)? != after {
                match after {
                    Some(bytes) => atomic_write(&target, bytes)?,
                    None => remove_optional(&target)?,
                }
            }
        }
        remove_optional(&path)
    }
}

fn refused(message: impl Into<String>) -> Response {
    Response::Refused {
        message: message.into(),
        report: None,
    }
}
fn io_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '\0'])
        && path.split('/').all(|part| {
            let stem = part
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
                && !(stem.len() == 4
                    && (stem.starts_with("com") || stem.starts_with("lpt"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
}

pub fn allowed_path(kind: WorkspaceKind, path: &str) -> bool {
    safe_path(path)
        && ((kind == WorkspaceKind::Project
            && (path == "scripts/lod-capture-manifest.toml"
                || path == "scripts/lod-manifest.toml"
                || (path.starts_with("scripts/art/lod-sources/") && assets::binary_path(path))))
            || crate::world::mod_pack::is_allowed_content_path(path)
            || (path.starts_with("assets/")
                && ((kind == WorkspaceKind::Project
                    && [".toml", ".rhai"]
                        .iter()
                        .any(|suffix| path.ends_with(suffix)))
                    || assets::binary_path(path))))
}

fn revision(files: &Files) -> String {
    let mut bytes = Vec::new();
    for (path, value) in files {
        bytes.extend((path.len() as u64).to_le_bytes());
        bytes.extend(path.as_bytes());
        bytes.extend((value.len() as u64).to_le_bytes());
        bytes.extend(value);
    }
    format!("{:016x}", vellum_digest::fnv1a(&bytes))
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}
fn remove_optional(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}
// Host-local temporary filename, never a simulation entity or replay identity.
#[allow(clippy::disallowed_methods)]
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing Workshop parent directory")?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let temporary = parent.join(format!(".phoenix-workshop-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        fs::rename(&temporary, path).map_err(io_error)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Random UUIDs isolate temporary test directories.
mod tests;
