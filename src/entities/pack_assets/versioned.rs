//! A new stack has new AssetServer identities. An old asynchronous completion
//! can finish only into its old handle, never into the replacement model.
use super::*;

pub(super) const SOURCE: &str = "phoenix-pack";

pub(super) struct VersionedReader(pub PackAssetReader);

fn split(path: &Path) -> Result<(u64, &Path), AssetReaderError> {
    let mut parts = path.components();
    let revision = parts
        .next()
        .and_then(|part| part.as_os_str().to_str())
        .and_then(|part| part.parse::<u64>().ok())
        .ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
    current(revision, path)?;
    Ok((revision, parts.as_path()))
}

fn current(revision: u64, path: &Path) -> Result<(), AssetReaderError> {
    if revision == mod_pack_revision() {
        Ok(())
    } else {
        Err(AssetReaderError::NotFound(path.to_owned()))
    }
}

impl ErasedAssetReader for VersionedReader {
    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<dyn Reader + 'a>, AssetReaderError>> {
        Box::pin(async move {
            let (revision, relative) = split(path)?;
            let mut source = self.0.read(relative).await?;
            let mut bytes = Vec::new();
            source.read_to_end(&mut bytes).await?;
            current(revision, path)?;
            Ok(Box::new(VecReader::new(bytes)) as Box<dyn Reader>)
        })
    }

    fn read_meta<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<dyn Reader + 'a>, AssetReaderError>> {
        Box::pin(async move {
            let bytes = self.read_meta_bytes(path).await?;
            Ok(Box::new(VecReader::new(bytes)) as Box<dyn Reader>)
        })
    }

    fn read_meta_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Vec<u8>, AssetReaderError>> {
        Box::pin(async move {
            let (revision, relative) = split(path)?;
            let bytes = self.0.read_meta_bytes(relative).await?;
            current(revision, path)?;
            Ok(bytes)
        })
    }

    fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<Box<PathStream>, AssetReaderError>> {
        Box::pin(async move {
            let (revision, relative) = split(path)?;
            let mut source = self.0.read_directory(relative).await?;
            let mut entries = Vec::new();
            while let Some(entry) = source.next().await {
                entries.push(Path::new(&revision.to_string()).join(entry));
            }
            current(revision, path)?;
            Ok(Box::new(stream::iter(entries)) as Box<PathStream>)
        })
    }

    fn is_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxedFuture<'a, Result<bool, AssetReaderError>> {
        Box::pin(async move {
            let (revision, relative) = split(path)?;
            let answer = self.0.is_directory(relative).await?;
            current(revision, path)?;
            Ok(answer)
        })
    }
}

/// The ordinary reader remains available to small fixtures and consumers that
/// do not install Phoenix's source. Production GLB/image loads use this source,
/// including their relative dependencies and generated labeled assets.
pub fn asset_path(server: &AssetServer, path: &str) -> String {
    let relative = path.strip_prefix("assets/").unwrap_or(path);
    if server.get_source(SOURCE).is_ok() {
        format!("{SOURCE}://{}/{relative}", mod_pack_revision())
    } else {
        relative.to_owned()
    }
}

/// Planet descriptors author paths from the asset root. Preserve the calling
/// descriptor's revision when resolving those paths, just as glTF does for its
/// relative image and buffer dependencies.
pub fn root_dependency(context: &bevy::asset::LoadContext<'_>, path: &str) -> String {
    if context.path().source() == &AssetSourceId::from(SOURCE) {
        if let Some(revision) = context.path().path().components().next() {
            return format!(
                "{SOURCE}://{}/{path}",
                revision.as_os_str().to_string_lossy()
            );
        }
    }
    path.to_owned()
}
