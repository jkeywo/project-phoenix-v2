//! Repository fixture access for this package's tests. Never changes process CWD.
//! Authored keys remain relative; only the final filesystem read is rooted.
use std::path::{Path, PathBuf};
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let first = path
        .components()
        .next()
        .and_then(|part| part.as_os_str().to_str());
    if !path.is_absolute()
        && matches!(
            first,
            Some("assets" | "tests" | "src" | "crates" | "pasm" | "gui" | "scripts" | "perf")
        )
    {
        root().join(path)
    } else {
        path.to_path_buf()
    }
}
pub mod fs {
    use std::{io, path::Path};
    pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
        std::fs::read_to_string(super::path(path))
    }
    pub fn read_dir(path: impl AsRef<Path>) -> io::Result<std::fs::ReadDir> {
        std::fs::read_dir(super::path(path))
    }
}
