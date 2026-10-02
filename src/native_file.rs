//! Native single-file replacement. Callers own serialization and multi-file recovery.
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const TEMP_EXTENSION: &str = "tmp";

pub(crate) enum TemporaryPolicy {
    Preference,
    Workshop,
}

impl TemporaryPolicy {
    // Host-local file identity; never simulation state.
    #[allow(clippy::disallowed_methods)]
    fn path(&self, path: &Path, parent: &Path) -> PathBuf {
        match self {
            Self::Preference => {
                let stem = path
                    .file_name()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or("layout".into());
                parent.join(format!("{stem}.{}.{TEMP_EXTENSION}", std::process::id()))
            }
            Self::Workshop => {
                parent.join(format!(".phoenix-workshop-{}.tmp", uuid::Uuid::new_v4()))
            }
        }
    }
}

/// Flush a sibling temporary before replacing the destination. Rename failures
/// (including Windows sharing violations) leave the previous destination intact.
/// Ordinary failures remove the temporary; callers own recovery after a hard kill.
pub(crate) fn replace(path: &Path, contents: &[u8], policy: TemporaryPolicy) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = policy.path(path, parent);
    let mut options = OpenOptions::new();
    options.write(true);
    match policy {
        TemporaryPolicy::Preference => {
            options.create(true).truncate(true);
        }
        TemporaryPolicy::Workshop => {
            options.create_new(true);
        }
    }
    // A failed create_new never grants ownership of an existing file.
    let mut file = options.open(&temporary)?;
    let written = file.write_all(contents).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn write_preferences(path: &Path, contents: &str) -> io::Result<()> {
    replace(path, contents.as_bytes(), TemporaryPolicy::Preference)
}

/// Recognise only the preference writer's owned crash debris, never arbitrary .tmp files.
pub(crate) fn names_a_temporary(name: &str) -> bool {
    let Some(rest) = name.strip_suffix(&format!(".{TEMP_EXTENSION}")) else {
        return false;
    };
    let Some((stem, pid)) = rest.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty() && !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
#[path = "native_file_tests.rs"]
mod tests;
