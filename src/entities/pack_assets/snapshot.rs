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
mod tests {
    use super::*;

    #[test]
    fn snapshot_sources_keep_exact_bytes_and_refuse_uncaptured_live_content() {
        let _guard = crate::entities::config_cache::overlay_test_guard();
        let captured = b"// captured shader\r\n".to_vec();
        let mut app = App::new();
        register(
            &mut app,
            Arc::new(BTreeMap::from([
                (
                    "assets/shaders/reference_grid.wgsl".into(),
                    Arc::from(captured.clone()),
                ),
                (
                    "assets/models/local/buffer.bin".into(),
                    Arc::from([0u8, 255, 13, 10]),
                ),
            ])),
        );
        // The ordinary boot registration must preserve explicitly selected
        // snapshot readers rather than reintroducing the live fallback.
        super::super::register(&mut app);
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ));
        let server = app.world().resource::<AssetServer>();
        for (source_id, prefix) in [
            (AssetSourceId::Default, String::new()),
            (
                AssetSourceId::from(versioned::SOURCE),
                format!("{}/", mod_pack_revision()),
            ),
        ] {
            let source = server.get_source(source_id).unwrap();
            let reader = source.reader();
            bevy::tasks::block_on(async {
                let shader_path = format!("{prefix}shaders/reference_grid.wgsl");
                let mut stream = reader.read(Path::new(&shader_path)).await.unwrap();
                let mut bytes = Vec::new();
                stream.read_to_end(&mut bytes).await.unwrap();
                assert_eq!(bytes, captured);
                assert!(
                    matches!(
                        reader
                            .read(Path::new(&format!(
                                "{prefix}entities/alliance_cruiser.toml"
                            )))
                            .await,
                        Err(AssetReaderError::NotFound(_))
                    ),
                    "shipped disk content must remain absent"
                );
                assert!(reader.read_meta(Path::new(&shader_path)).await.is_err());
                let mut directory = reader
                    .read_directory(Path::new(&format!("{prefix}models")))
                    .await
                    .unwrap();
                assert_eq!(
                    directory.next().await,
                    Some(Path::new(&format!("{prefix}models")).join("local"))
                );
                assert!(directory.next().await.is_none());
                assert!(reader
                    .is_directory(Path::new(&format!("{prefix}models/local")))
                    .await
                    .unwrap());
                assert!(reader
                    .read(Path::new(&format!("{prefix}../outside")))
                    .await
                    .is_err());
            });
        }
        let versioned = server.get_source(versioned::SOURCE).unwrap();
        assert!(bevy::tasks::block_on(versioned.reader().read(Path::new(
            "18446744073709551615/shaders/reference_grid.wgsl"
        )))
        .is_err());
    }
}
