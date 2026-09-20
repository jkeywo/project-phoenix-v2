//! Freeze exactly the validated unsaved document and its immutable dependencies.
//! The snapshot has no native source-root path and cannot write back to one.
use super::{
    assets::Sources as DraftSources, Files, NativeWorkshopProvider, Response, WorkspaceKind,
};
pub use crate::workshop::test_protocol::TestSelection;
pub use crate::workshop::test_source::TestCatalog;
use crate::workshop::Sources;

#[derive(Debug)]
pub struct TestSnapshot {
    pub files: Files,
    pub selection: TestSelection,
    pub revision: String,
}

/// Runtime-owned render support is captured read-only. This does not widen the
/// editable source or Mod asset allowlist to shader code.
pub fn runtime_support_path(path: &str) -> bool {
    super::safe_path(path) && path.starts_with("assets/shaders/") && path.ends_with(".wgsl")
}

pub(crate) fn stage_path(path: &str) -> bool {
    super::allowed_path(WorkspaceKind::Project, path) || runtime_support_path(path)
}

impl NativeWorkshopProvider {
    pub(super) fn prepare_runtime_capture(
        &self,
        sources: DraftSources,
        refusal: &'static str,
    ) -> Result<Files, Response> {
        let files = self.assets.materialize(sources).map_err(super::refused)?;
        self.check_files(&files).map_err(super::refused)?;
        let report = self.validate(&files);
        if !report.accepted {
            return Err(Response::Refused {
                message: refusal.into(),
                report: Some(report),
            });
        }
        let mut merged = self.test_support.clone();
        if self.kind == WorkspaceKind::Mod {
            merged.extend(
                self.dependencies
                    .base_files
                    .iter()
                    .map(|(path, text)| (path.clone(), text.as_bytes().to_vec())),
            );
            merged.extend(self.dependencies.base_assets.clone());
            for pack in &self.dependencies.packs {
                merged.extend(
                    pack.files
                        .iter()
                        .map(|(path, text)| (path.clone(), text.as_bytes().to_vec())),
                );
                merged.extend(pack.assets.clone());
            }
        }
        merged.extend(files);
        let mut folded = std::collections::BTreeSet::new();
        if merged.len() > super::MAX_FILES
            || merged.values().map(Vec::len).sum::<usize>() > super::MAX_BYTES
            || merged
                .keys()
                .any(|path| !stage_path(path) || !folded.insert(path.to_ascii_lowercase()))
        {
            return Err(super::refused(
                "Workshop runtime snapshot exceeds the supported content envelope",
            ));
        }
        Ok(merged)
    }

    pub(super) fn capture_test_support(&self) -> Result<Files, String> {
        fn walk(
            provider: &NativeWorkshopProvider,
            directory: &std::path::Path,
            files: &mut Files,
        ) -> Result<(), String> {
            for entry in std::fs::read_dir(directory).map_err(super::io_error)? {
                let entry = entry.map_err(super::io_error)?;
                let name = entry
                    .path()
                    .strip_prefix(&provider.root)
                    .map_err(super::io_error)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let path = provider.resolve(&name)?;
                if entry.file_type().map_err(super::io_error)?.is_dir() {
                    walk(provider, &path, files)?;
                } else if runtime_support_path(&name) {
                    use std::io::Read;
                    let mut bytes = Vec::new();
                    std::fs::File::open(path)
                        .map_err(super::io_error)?
                        .take(4 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(super::io_error)?;
                    files.insert(name, bytes);
                    if files.len() > 128
                        || files.values().map(Vec::len).sum::<usize>() > 4 * 1024 * 1024
                    {
                        return Err("Workshop render support is too large".into());
                    }
                }
            }
            Ok(())
        }
        let mut files = Files::new();
        if self.kind == WorkspaceKind::Project {
            let directory = self.resolve("assets/shaders")?;
            if directory.is_dir() {
                walk(self, &directory, &mut files)?;
            }
        }
        Ok(files)
    }
    /// Choices are derived from composed source, including read-only base and
    /// dependency hulls. Listing never materializes a binary asset version.
    pub fn test_catalog(
        &self,
        files: std::collections::BTreeMap<String, String>,
    ) -> Result<TestCatalog, String> {
        if files.len() > super::MAX_FILES
            || files.values().map(String::len).sum::<usize>() > super::MAX_BYTES
            || files.keys().any(|path| {
                !super::allowed_path(self.kind, path)
                    || !(path.ends_with(".toml") || path.ends_with(".rhai"))
            })
        {
            return Err("Invalid Test catalogue source".into());
        }
        let mut text = std::collections::BTreeMap::new();
        if self.kind == WorkspaceKind::Mod {
            text.extend(self.dependencies.base_files.clone());
            for pack in &self.dependencies.packs {
                text.extend(pack.files.clone());
            }
        }
        text.extend(files);
        Ok(crate::workshop::test_source::catalog(text))
    }

    pub fn prepare_test(
        &self,
        sources: DraftSources,
        selection: TestSelection,
    ) -> Result<TestSnapshot, Response> {
        let merged = self.prepare_runtime_capture(sources, "Runtime validation refused Test")?;
        let text = Sources(
            merged
                .iter()
                .filter_map(|(path, bytes)| {
                    (path.ends_with(".toml") || path.ends_with(".rhai"))
                        .then(|| {
                            String::from_utf8(bytes.clone())
                                .ok()
                                .map(|text| (path.clone(), text))
                        })
                        .flatten()
                })
                .collect(),
        );
        let report = crate::workshop::test_source::validate_selection(text.0, &selection);
        if !report.accepted {
            return Err(Response::Refused {
                message: "Runtime validation refused the Test selection".into(),
                report: Some(report),
            });
        }
        Ok(TestSnapshot {
            revision: super::revision(&merged),
            files: merged,
            selection,
        })
    }
}
