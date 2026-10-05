//! Immutable byte versions private to one selected Workshop root. The UI sees
//! bounded references and chunks; authored paths never name a file to read here.
use super::{atomic_write, io_error, Files, MAX_BYTES, MAX_FILES};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

pub const CHUNK_BYTES: usize = 64 * 1024;
const MAX_STORED_BYTES: u64 = 4 * MAX_BYTES as u64;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetReference {
    pub asset: String,
    pub length: usize,
}
impl AssetReference {
    fn for_bytes(bytes: &[u8]) -> Self {
        Self {
            asset: format!("{:016x}-{}", vellum_digest::fnv1a(bytes), bytes.len()),
            length: bytes.len(),
        }
    }
    fn valid(&self) -> bool {
        let Some((hash, length)) = self.asset.split_once('-') else {
            return false;
        };
        self.length <= MAX_BYTES
            && hash.len() == 16
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && length == self.length.to_string()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Source {
    Text(String),
    Bytes(Vec<u8>),
    Asset(AssetReference),
}
pub type Sources = BTreeMap<String, Source>;

pub fn binary_path(path: &str) -> bool {
    [
        ".glb", ".png", ".jpg", ".jpeg", ".ktx2", ".ptex", ".wav", ".ogg", ".mp3",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
        || (path.starts_with("assets/models/") && path.ends_with(".bin"))
}

struct Upload {
    token: String,
    length: usize,
    bytes: Vec<u8>,
}
pub struct AssetStore {
    directory: PathBuf,
    upload: Option<Upload>,
    // One currently previewed member, so sequential chunk reads do not re-read
    // and hash a large model for each 64 KiB message. Saves always verify anew.
    preview: Option<(AssetReference, Vec<u8>)>,
}
impl AssetStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            upload: None,
            preview: None,
        }
    }

    fn path(&self, reference: &AssetReference) -> Result<PathBuf, String> {
        if !reference.valid() {
            return Err("Invalid native asset reference".into());
        }
        Ok(self.directory.join(format!("{}.blob", reference.asset)))
    }
    fn read(&self, reference: &AssetReference) -> Result<Vec<u8>, String> {
        let path = self.path(reference)?;
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != reference.length as u64
        {
            return Err("Native asset version is missing or changed".into());
        }
        let bytes = fs::read(path).map_err(io_error)?;
        if AssetReference::for_bytes(&bytes) != *reference {
            return Err("Native asset version is changed".into());
        }
        Ok(bytes)
    }
    pub fn store(&self, bytes: &[u8]) -> Result<AssetReference, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Workshop asset is too large".into());
        }
        let reference = AssetReference::for_bytes(bytes);
        let path = self.path(&reference)?;
        if path.exists() {
            // A content-key collision or damaged private file must never bless
            // different bytes as the old version, even with the same digest.
            if self.read(&reference)? != bytes {
                return Err("Native asset version conflicts".into());
            }
        } else {
            fs::create_dir_all(&self.directory).map_err(io_error)?;
            let mut stored = 0u64;
            for entry in fs::read_dir(&self.directory).map_err(io_error)? {
                stored = stored
                    .saturating_add(entry.map_err(io_error)?.metadata().map_err(io_error)?.len());
            }
            if stored.saturating_add(bytes.len() as u64) > MAX_STORED_BYTES {
                return Err("Workshop private asset recovery storage is full".into());
            }
            atomic_write(&path, bytes)?;
        }
        Ok(reference)
    }
    pub fn compact(&self, files: &Files) -> Result<Sources, String> {
        files
            .iter()
            .map(|(path, bytes)| {
                Ok((
                    path.clone(),
                    if binary_path(path) {
                        Source::Asset(self.store(bytes)?)
                    } else {
                        Source::Text(String::from_utf8(bytes.clone()).map_err(io_error)?)
                    },
                ))
            })
            .collect()
    }
    pub fn materialize(&self, sources: Sources) -> Result<Files, String> {
        if sources.len() > MAX_FILES {
            return Err("Workshop source bundle is too large".into());
        }
        let mut total = 0usize;
        let mut files = Files::new();
        for (path, source) in sources {
            let length = match &source {
                Source::Text(text) => text.len(),
                Source::Bytes(bytes) => bytes.len(),
                Source::Asset(asset) => asset.length,
            };
            total = total
                .checked_add(length)
                .ok_or("Workshop source bundle is too large")?;
            if total > MAX_BYTES {
                return Err("Workshop source bundle is too large".into());
            }
            let bytes = match source {
                Source::Text(text) if !binary_path(&path) => text.into_bytes(),
                Source::Text(_) => return Err("Native asset bytes cannot be source text".into()),
                Source::Bytes(bytes) => bytes,
                Source::Asset(reference) if binary_path(&path) => self.read(&reference)?,
                Source::Asset(_) => {
                    return Err("Native asset reference cannot replace source text".into())
                }
            };
            files.insert(path, bytes);
        }
        Ok(files)
    }
    pub fn read_chunk(
        &mut self,
        reference: AssetReference,
        offset: usize,
    ) -> Result<Vec<u8>, String> {
        if offset > reference.length {
            return Err("Invalid native asset offset".into());
        }
        if !self
            .preview
            .as_ref()
            .is_some_and(|(current, _)| current == &reference)
        {
            let bytes = self.read(&reference)?;
            self.preview = Some((reference.clone(), bytes));
        }
        let bytes = &self.preview.as_ref().ok_or("Missing native asset")?.1;
        Ok(bytes[offset..bytes.len().min(offset.saturating_add(CHUNK_BYTES))].to_vec())
    }
    // Private upload capability, never a simulation entity or replay identity.
    #[allow(clippy::disallowed_methods)]
    pub fn begin(&mut self, length: usize) -> Result<String, String> {
        if length > MAX_BYTES || self.upload.is_some() {
            return Err("Workshop asset upload is unavailable".into());
        }
        let token = uuid::Uuid::new_v4().to_string();
        self.upload = Some(Upload {
            token: token.clone(),
            length,
            bytes: Vec::new(),
        });
        Ok(token)
    }
    pub fn append(&mut self, token: &str, offset: usize, bytes: &[u8]) -> Result<(), String> {
        let upload = self.upload.as_mut().ok_or("No Workshop asset upload")?;
        if upload.token != token
            || offset != upload.bytes.len()
            || bytes.len() > CHUNK_BYTES
            || upload.bytes.len().saturating_add(bytes.len()) > upload.length
        {
            return Err("Invalid Workshop asset chunk".into());
        }
        upload.bytes.extend_from_slice(bytes);
        Ok(())
    }
    pub fn finish(&mut self, token: &str) -> Result<AssetReference, String> {
        let upload = self.upload.as_ref().ok_or("No Workshop asset upload")?;
        if upload.token != token || upload.bytes.len() != upload.length {
            return Err("Incomplete Workshop asset upload".into());
        }
        let reference = self.store(&upload.bytes)?;
        self.upload = None;
        Ok(reference)
    }
    pub fn cancel(&mut self, token: &str) -> Result<(), String> {
        if self
            .upload
            .as_ref()
            .is_some_and(|upload| upload.token != token)
        {
            return Err("Unknown Workshop asset upload".into());
        }
        self.upload = None;
        Ok(())
    }
    pub(super) fn retire_view(&mut self) {
        self.upload = None;
        self.preview = None;
    }
}
