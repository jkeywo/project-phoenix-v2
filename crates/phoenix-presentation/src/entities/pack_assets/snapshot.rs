//! A disposable Test reads one immutable per-App source bundle. There is no
//! disk, HTTP, active-pack cache or metadata fallback behind these readers.
use super::*;
use std::{collections::BTreeMap, sync::Arc};

pub type SnapshotAssets = Arc<BTreeMap<String, Arc<[u8]>>>;

#[derive(Resource)]
struct Installed;

pub(super) fn installed(app: &App) -> bool {
    app.world().contains_resource::<Installed>()
}

pub(super) fn register(app: &mut App, files: SnapshotAssets) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<Installed>(StateClass::Derived, "gm-milestone-integrated-workshop")
        .insert_resource(Installed);
    let ordinary = files.clone();
    app.register_asset_source(
        AssetSourceId::Default,
        AssetSourceBuilder::new(move || {
            Box::new(SnapshotReader {
                files: ordinary.clone(),
                revision: None,
            })
        }),
    );
    // asset_path uses the revision prefix even in Test. Capture it once: the
    // reader never follows a later live overlay or changes its source bytes.
    let revision = mod_pack_revision();
    app.register_asset_source(
        versioned::SOURCE,
        AssetSourceBuilder::new(move || {
            Box::new(SnapshotReader {
                files: files.clone(),
                revision: Some(revision),
            })
        }),
    );
}

struct SnapshotReader {
    files: SnapshotAssets,
    revision: Option<u64>,
}

impl SnapshotReader {
    fn authored(&self, path: &Path) -> Result<String, AssetReaderError> {
        let missing = || AssetReaderError::NotFound(path.to_owned());
        let relative = if let Some(expected) = self.revision {
            let mut parts = path.components();
            let revision = parts
                .next()
                .and_then(|part| part.as_os_str().to_str())
                .and_then(|part| part.parse::<u64>().ok());
            if revision != Some(expected) {
                return Err(missing());
            }
            parts.as_path()
        } else {
            path
        };
        authored_path(relative).ok_or_else(missing)
    }
}

impl ErasedAssetReader for SnapshotReader {
    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<dyn Reader + 'a>, AssetReaderError>> {
        Box::pin(async move {
            let bytes = self
                .files
                .get(&self.authored(path)?)
                .ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
            Ok(Box::new(VecReader::new(bytes.to_vec())) as Box<dyn Reader>)
        })
    }

    fn read_meta<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<dyn Reader + 'a>, AssetReaderError>> {
        Box::pin(async move { Err(AssetReaderError::NotFound(path.to_owned())) })
    }

    fn read_meta_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Vec<u8>, AssetReaderError>> {
        Box::pin(async move { Err(AssetReaderError::NotFound(path.to_owned())) })
    }

    fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<PathStream>, AssetReaderError>> {
        Box::pin(async move {
            let prefix = format!("{}/", self.authored(path)?);
            let entries: std::collections::BTreeSet<_> = self
                .files
                .keys()
                .filter_map(|name| name.strip_prefix(&prefix))
                .filter_map(|tail| tail.split('/').next().filter(|part| !part.is_empty()))
                .map(|child| path.join(child))
                .collect();
            if entries.is_empty() {
                return Err(AssetReaderError::NotFound(path.to_owned()));
            }
            Ok(Box::new(stream::iter(entries)) as Box<PathStream>)
        })
    }

    fn is_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<bool, AssetReaderError>> {
        Box::pin(async move {
            let authored = self.authored(path)?;
            if self.files.contains_key(&authored) {
                return Ok(false);
            }
            let prefix = format!("{authored}/");
            if self.files.keys().any(|name| name.starts_with(&prefix)) {
                return Ok(true);
            }
            Err(AssetReaderError::NotFound(path.to_owned()))
        })
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
