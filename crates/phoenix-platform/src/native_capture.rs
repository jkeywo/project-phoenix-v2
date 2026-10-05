//! Native directory capture mechanics; callers own admission and check timing.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Visit entries in the filesystem's order and descend only where admitted.
pub fn walk(
    directory: &Path,
    visit: &mut impl FnMut(fs::DirEntry) -> Result<Option<PathBuf>, String>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        if let Some(next) = visit(entry.map_err(|e| e.to_string())?)? {
            walk(&next, visit)?;
        }
    }
    Ok(())
}

pub fn relative_name(path: &Path, root: &Path) -> Result<String, String> {
    Ok(path
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/"))
}

/// Read one byte past the caller's limit so its existing refusal can detect it.
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Refuse at the caller's existing count/byte checkpoint.
pub fn check_size(
    count: usize,
    bytes: usize,
    max_count: usize,
    max_bytes: usize,
    refusal: &str,
) -> Result<(), String> {
    if count > max_count || bytes > max_bytes {
        Err(refusal.into())
    } else {
        Ok(())
    }
}

/// Editable and Test captures check the resulting map after inserting a member.
pub fn insert_checked(
    files: &mut std::collections::BTreeMap<String, Vec<u8>>,
    name: String,
    bytes: Vec<u8>,
    max_count: usize,
    max_bytes: usize,
    refusal: &str,
) -> Result<(), String> {
    files.insert(name, bytes);
    if files.len() > max_count {
        return Err(refusal.into());
    }
    check_size(
        files.len(),
        files.values().map(Vec::len).sum(),
        max_count,
        max_bytes,
        refusal,
    )
}

#[cfg(test)]
#[path = "native_capture_tests.rs"]
mod tests;
