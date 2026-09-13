//! Render reads use the same immutable accepted pack stack as authored source.
//! The default disk/HTTP reader remains the fallback for shipped content.
use std::path::Path;

use bevy::{
    asset::{
        io::{
            AssetReaderError, AssetSource, AssetSourceBuilder, AssetSourceId, ErasedAssetReader,
            PathStream, Reader, VecReader,
        },
        AssetApp, AssetServer,
    },
    prelude::*,
    tasks::{
        futures_lite::{stream, StreamExt},
        BoxedFuture,
    },
};

use super::config_cache::{mod_pack_asset, mod_pack_assets, mod_pack_revision};

#[cfg(test)]
mod runtime_tests;
mod snapshot;
mod versioned;
pub use snapshot::SnapshotAssets;
pub use versioned::{asset_path, root_dependency};

/// Bind a disposable App to exact captured bytes before adding AssetPlugin.
pub fn register_snapshot(app: &mut App, files: SnapshotAssets) {
    snapshot::register(app, files);
}

/// A child owned by the authored render adapter, safe to retire with its asset
/// revision without touching any simulation entity or gameplay child.
#[derive(Component)]
pub struct PackVisualRoot;

pub struct PackAssetReader {
    fallback: Box<dyn ErasedAssetReader>,
}

impl PackAssetReader {
    pub fn new(fallback: Box<dyn ErasedAssetReader>) -> Self {
        Self { fallback }
    }
}

fn authored_path(path: &Path) -> Option<String> {
    let mut parts = vec!["assets"];
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                let part = part.to_str()?;
                if part.contains(['\\', ':']) || part.chars().any(char::is_control) {
                    return None;
                }
                parts.push(part);
            }
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    // Path::join uses the native separator for a glTF dependency or directory
    // child. Accepted pack keys use portable slashes on every platform.
    Some(parts.join("/"))
}

impl ErasedAssetReader for PackAssetReader {
    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<dyn Reader + 'a>, AssetReaderError>> {
        Box::pin(async move {
            let authored =
                authored_path(path).ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
            if let Some(bytes) = mod_pack_asset(&authored) {
                return Ok(Box::new(VecReader::new(bytes.to_vec())) as Box<dyn Reader>);
            }
            self.fallback.read(path).await
        })
    }

    fn read_meta<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<dyn Reader + 'a>, AssetReaderError>> {
        Box::pin(async move {
            let authored =
                authored_path(path).ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
            if mod_pack_asset(&authored).is_some() {
                // A replacement model must not inherit a shipped file's loader
                // settings. Uploaded metadata files are outside the pack format.
                return Err(AssetReaderError::NotFound(path.to_owned()));
            }
            self.fallback.read_meta(path).await
        })
    }

    fn read_meta_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Vec<u8>, AssetReaderError>> {
        Box::pin(async move {
            let mut reader = self.read_meta(path).await?;
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await?;
            Ok(bytes)
        })
    }

    fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<PathStream>, AssetReaderError>> {
        Box::pin(async move {
            let prefix =
                authored_path(path).ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
            let prefix = format!("{}/", prefix.trim_end_matches('/'));
            let mut entries = std::collections::BTreeSet::new();
            for asset in mod_pack_assets().keys() {
                if let Some(tail) = asset.strip_prefix(&prefix) {
                    if let Some(child) = tail.split('/').next().filter(|s| !s.is_empty()) {
                        entries.insert(path.join(child));
                    }
                }
            }
            match self.fallback.read_directory(path).await {
                Ok(mut source) => {
                    while let Some(entry) = source.next().await {
                        entries.insert(entry);
                    }
                }
                Err(AssetReaderError::NotFound(_)) if !entries.is_empty() => {}
                Err(error) => return Err(error),
            }
            Ok(Box::new(stream::iter(entries)) as Box<PathStream>)
        })
    }

    fn is_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<bool, AssetReaderError>> {
        Box::pin(async move {
            let prefix =
                authored_path(path).ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
            let prefix = format!("{}/", prefix.trim_end_matches('/'));
            if mod_pack_assets()
                .keys()
                .any(|asset| asset.starts_with(&prefix))
            {
                return Ok(true);
            }
            self.fallback.is_directory(path).await
        })
    }
}

/// Must run before AssetPlugin, in every native/browser composition path.
pub fn register(app: &mut App) {
    if snapshot::installed(app) {
        return;
    }
    let mut fallback = AssetSource::get_default_reader("assets".into());
    app.register_asset_source(
        AssetSourceId::Default,
        AssetSourceBuilder::new(move || Box::new(PackAssetReader::new(fallback()))),
    );
    let mut fallback = AssetSource::get_default_reader("assets".into());
    app.register_asset_source(
        versioned::SOURCE,
        AssetSourceBuilder::new(move || {
            Box::new(versioned::VersionedReader(PackAssetReader::new(fallback())))
        }),
    );
    // Native pack choices arrive during Update, alongside render adapters.
    // Retire every queued old visual after those commands have applied and
    // before extraction; otherwise a loaded old handle could draw once more.
    let mut previous = mod_pack_revision();
    app.add_systems(Last, move |world: &mut World| {
        refresh_loaded_assets(world, &mut previous);
    });
}

fn refresh_loaded_assets(world: &mut World, previous: &mut u64) {
    let revision = mod_pack_revision();
    if *previous == revision {
        return;
    }
    crate::server_app_render::reset_pack_visuals(world);
    #[cfg(feature = "server")]
    crate::server::asset_preload::reset_pack_preloads(world);
    #[cfg(feature = "server")]
    crate::server::pfx::reset_pack_textures(world);
    #[cfg(feature = "viewer")]
    crate::viewer::reset_pack_visuals(world);
    *previous = revision;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::config_cache::{
        push_mod_pack, remove_mod_pack, reorder_mod_packs, ActivePack,
    };
    use std::sync::Arc;

    #[test]
    fn native_dependency_paths_resolve_to_portable_pack_keys() {
        let dependency = Path::new("models").join("probe").join("texture.png");
        assert_eq!(
            authored_path(&dependency).as_deref(),
            Some("assets/models/probe/texture.png")
        );
        assert_eq!(authored_path(Path::new("")).as_deref(), Some("assets"));
        assert!(authored_path(Path::new("../models/escape.glb")).is_none());
        assert!(authored_path(Path::new("/models/escape.glb")).is_none());
        #[cfg(windows)]
        {
            assert_eq!(
                authored_path(Path::new(r"models\probe\texture.png")).as_deref(),
                Some("assets/models/probe/texture.png")
            );
            assert!(authored_path(Path::new(r"C:\models\escape.glb")).is_none());
        }
    }

    #[test]
    fn asset_reads_follow_stack_reorder_remove_and_disk_fallback_without_utf8_conversion() {
        let _guard = crate::entities::config_cache::overlay_test_guard();
        let memory = bevy::asset::io::memory::Dir::default();
        memory.insert_asset(Path::new("models/probe.glb"), vec![1, 2, 3]);
        let reader = PackAssetReader::new(Box::new(bevy::asset::io::memory::MemoryAssetReader {
            root: memory,
        }));
        let path = "assets/models/probe.glb";
        let install = |id: &str, bytes: &[u8]| {
            push_mod_pack(ActivePack {
                id: id.into(),
                assets: [(path.into(), Arc::from(bytes))].into(),
                ..default()
            })
        };
        let read = || {
            bevy::tasks::block_on(async {
                let mut stream = reader.read(Path::new("models/probe.glb")).await.unwrap();
                let mut bytes = Vec::new();
                stream.read_to_end(&mut bytes).await.unwrap();
                bytes
            })
        };
        assert_eq!(read(), [1, 2, 3]);
        install("first", &[0, 255, 1]);
        install("second", &[254, 0, 2]);
        assert_eq!(read(), [254, 0, 2]);
        reorder_mod_packs(&["second".into(), "first".into()]);
        assert_eq!(read(), [0, 255, 1]);
        remove_mod_pack("first");
        assert_eq!(read(), [254, 0, 2]);
        remove_mod_pack("second");
        assert_eq!(read(), [1, 2, 3]);
    }
}
