//! The **saved bridge layouts** (issue #1334) — Bevy-free, and its location is
//! injected.
//!
//! [`super::bridge_profile`] is the file an operator *writes*.
//! [`super::bridge_layout`] is the law a running bridge obeys. This is the file
//! the host writes *for* the operator: the arrangement they built in the lobby,
//! filed under the ship class they built it for, so that picking that class
//! again next week puts the viewscreen back on the monitor they chose and
//! reopens each assigned console on the screen they put it on.
//!
//! ```text
//! %APPDATA%\ProjectPhoenix\bridge-layouts\alliance_destroyer.toml
//! ```
//!
//! # Three properties, and each one is a rule rather than an implementation
//!
//! 1. **The location is injectable.** A [`LayoutStore`] is a directory and
//!    nothing else. [`LayoutStore::user`] is the one that resolves the
//!    operator's real settings directory; every other constructor takes a path,
//!    which is what lets the tests below run against a scratch directory and
//!    never touch a developer's own bridge.
//! 2. **A write is atomic.** [`LayoutStore::save`] writes a temporary file
//!    beside the target, flushes it to the device, and renames it over the top —
//!    so a host killed mid-write leaves either the old layout or the new one,
//!    never half a TOML file that the next boot then reports as corrupt. See
//!    [`write_atomically`] for what that means on Windows specifically.
//! 3. **A saved layout is never trusted.** [`LayoutStore::load`] parses *and*
//!    re-validates, and hands back a [`ValidatedProfile`] — the same type
//!    [`BridgeLayout::adopt_profile`] takes from a hand-authored `--profile`,
//!    so a file that was edited by hand, written by an older build, or
//!    truncated by a full disk meets exactly the refusals a `--profile` meets.
//!    The caller's answer to that is a warning and today's bridge, never a
//!    failed boot (issue #1334's acceptance criterion).
//!
//! # What may be saved: lobby-built layouts, and only those
//!
//! [`save`](LayoutStore::save) **refuses** a layout that carries reservations —
//! the authored `--pane` surfaces a `--profile` opened, which
//! [`BridgeLayout::reserved_on`] records and
//! [`BridgeLayout::to_validated_profile`] does not re-emit.
//!
//! That refusal is this module's half of a data-loss guard whose other half is
//! the *policy* in [`super::layout_store_systems`]: a `--profile` run never
//! writes at all. Either alone would do it today; both are here because the
//! failure they prevent is silent and permanent. Writing a `--profile`-seeded
//! layout would drop the operator's `[[display.pane]]` participant slots out of
//! the file, and the *next* boot would read those screens as free and move the
//! viewscreen on top of a crew member's live console — the exact failure
//! `reserved` exists to prevent, re-introduced by the save. A layout the lobby
//! built is reservation-free by construction (nothing but
//! [`BridgeLayout::adopt_profile`] ever populates `reserved`, and nothing but an
//! authored `--profile` reaches it), so this refusal costs a correct caller
//! nothing and is unreachable from the shipped path — which is precisely the
//! property worth pinning with a test.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::bridge_layout::BridgeLayout;
use super::bridge_profile::{BridgeProfile, ProfileError, ValidatedProfile};

/// The directory the saved layouts live under, inside the operator's data
/// directory. `%APPDATA%\ProjectPhoenix` on Windows.
pub const APP_DIR: &str = "ProjectPhoenix";

/// The saved-layout directory itself, inside [`APP_DIR`].
pub const LAYOUTS_DIR: &str = "bridge-layouts";

/// The extension a saved layout carries — the same TOML a `--profile` is
/// written in, because it *is* one: an operator who opens the file finds the
/// shape [`super::bridge_profile`] documents, and may hand it straight to
/// `--profile`.
pub const LAYOUT_EXTENSION: &str = "toml";

// ── the class key ───────────────────────────────────────────────────────────

/// The filing key for one ship class: the stem of its hull template's canonical
/// path, lowercased and reduced to a filename that cannot mean anything else.
///
/// # Why the template path, and not something on the hull
///
/// [ai] There is no authored class identity to key on. [`crate::ship::config::ShipConfig`]
/// carries stations, systems and power groups and no class id, so the only
/// stable name a hull answers to is the template it was loaded from — which is
/// exactly what [`crate::lobby::SelectedShipResource`] already holds, on both
/// world-arrival paths, in the **canonical** form
/// [`canonical_template_path`](crate::entities::include_resolve::canonical_template_path)
/// produced (`app::install_world_selection` stores the canonical key
/// deliberately, so `./assets\entities\x.toml` and `assets/entities/x.toml` are
/// one hull rather than two).
///
/// The *stem* rather than the whole path because the directory this names is one
/// an operator opens: `alliance_destroyer.toml` beside `alliance_cruiser.toml`
/// is a directory somebody can read, delete one entry from, or copy between
/// machines, and `assets_entities_alliance_destroyer.toml` is not.
///
/// The trade that buys: two hulls with the same file name in different
/// directories — a mod pack shipping its own `alliance_destroyer.toml` — share
/// one saved layout. That is a **degradation, not a fault**: the layout is
/// adopted through [`BridgeLayout::reconcile`] against the hull that is actually
/// flying, so stations the other hull does not have simply arrive unassigned
/// and are reported, which is the same graceful answer a changed monitor set
/// gets. It is recorded here rather than fixed by hashing the full path,
/// because a readable directory is the feature and a shared layout between two
/// hulls of the same name is a mild surprise rather than a lost arrangement.
///
/// # It cannot name a file outside the store
///
/// Every character outside `[a-z0-9_-]` becomes `_`, so a key can hold no path
/// separator, no drive letter and no `..`; a stem that reduces to nothing but
/// separators (`..`, `.`, the empty string) is `None` rather than a key. That
/// is why this is a newtype and why [`LayoutStore`] takes one instead of a
/// `&str`: there is no way to hand the store a class name it has not been
/// through.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShipClassKey(String);

impl ShipClassKey {
    /// The key for the hull at `template_path`, or `None` when the path names
    /// no usable file stem.
    pub fn from_template_path(template_path: &str) -> Option<Self> {
        let path = template_path.replace('\\', "/").to_ascii_lowercase();
        let name = path.rsplit('/').next().unwrap_or_default();
        // Lowercased BEFORE the extension is stripped, so a hand-typed
        // `--ship …\Alliance_Destroyer.TOML` files under the same key the
        // canonical path does rather than under `alliance_destroyer_toml`.
        let stem = name
            .strip_suffix(&format!(".{LAYOUT_EXTENSION}"))
            .unwrap_or(name);
        let key: String = stem
            .chars()
            .map(|c| match c {
                'a'..='z' | '0'..='9' | '_' | '-' => c,
                _ => '_',
            })
            .collect();
        // A stem that was nothing but separators names no class. `..` and `.`
        // both land here, which is what makes traversal unrepresentable rather
        // than merely filtered.
        if key.is_empty() || key.chars().all(|c| c == '_') {
            return None;
        }
        Some(Self(key))
    }

    /// The key as it appears in the file name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ShipClassKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── the store ───────────────────────────────────────────────────────────────

/// A directory of per-ship-class bridge layouts.
///
/// Holds no state beyond its root, so two stores on one directory are the same
/// store and a test may build as many as it likes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutStore {
    root: PathBuf,
}

impl LayoutStore {
    /// A store rooted at `root`. The directory need not exist — the first
    /// [`save`](Self::save) creates it.
    ///
    /// This is the injection point: the tests hand it a scratch directory, and
    /// nothing in this module ever consults the environment except
    /// [`user`](Self::user).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The operator's own store —
    /// `<data dir>/`[`APP_DIR`]`/`[`LAYOUTS_DIR`], which on Windows is
    /// `%APPDATA%\ProjectPhoenix\bridge-layouts`.
    ///
    /// `None` when there is no home directory to resolve (a service account, a
    /// stripped container). The caller's answer to that is to run without saved
    /// layouts, not to fail: a bridge nobody can remember is still a bridge.
    ///
    /// [ai] `BaseDirs::data_dir()` rather than `ProjectDirs`: `ProjectDirs`
    /// appends its own `data`/`config` leaf on Windows
    /// (`%APPDATA%\<Org>\<App>\data`), which is not the path this issue
    /// specifies and is not a directory an operator would look in.
    /// `data_dir()` is `%APPDATA%` exactly, and the two components below are
    /// this crate's own — so the Windows path is the authored one, and Linux
    /// (`~/.local/share`) and macOS (`~/Library/Application Support`) get their
    /// own conventional root for free rather than a Windows-shaped one.
    pub fn user() -> Option<Self> {
        let base = directories::BaseDirs::new()?;
        Some(Self::at(base.data_dir().join(APP_DIR).join(LAYOUTS_DIR)))
    }

    /// The directory this store reads and writes.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where `class`'s layout is filed.
    pub fn path_for(&self, class: &ShipClassKey) -> PathBuf {
        self.root
            .join(format!("{}.{LAYOUT_EXTENSION}", class.as_str()))
    }

    /// File `layout` as `class`'s remembered bridge, creating the directory on
    /// the first write. Answers the path it wrote.
    ///
    /// Refuses [`LayoutStoreError::WouldDropReservations`] for a layout carrying
    /// authored `--pane` surfaces — see the [module note](self#what-may-be-saved-lobby-built-layouts-and-only-those).
    /// The check is made *here*, at the only door onto the disk, rather than at
    /// the caller: a second caller added later inherits the guard instead of
    /// having to remember it.
    ///
    /// What is written is [`BridgeLayout::to_profile`], which is the same
    /// content [`BridgeLayout::to_validated_profile`] proves valid — that
    /// conversion is infallible by construction, so there is no validity error
    /// to report from this side. [`load`](Self::load) re-validates anyway,
    /// because by then the file has been on a disk an operator can edit.
    pub fn save(
        &self,
        class: &ShipClassKey,
        layout: &BridgeLayout,
    ) -> Result<PathBuf, LayoutStoreError> {
        let reserved = reservations(layout);
        if !reserved.is_empty() {
            return Err(LayoutStoreError::WouldDropReservations {
                class: class.to_string(),
                labels: reserved,
            });
        }
        let path = self.path_for(class);
        let text = layout
            .to_profile()
            .to_toml()
            .map_err(|source| LayoutStoreError::Profile {
                path: path.display().to_string(),
                source,
            })?;
        write_atomically(&path, &text).map_err(|e| LayoutStoreError::Write {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
        Ok(path)
    }

    /// `class`'s remembered bridge, re-validated.
    ///
    /// `Ok(None)` when there is no file — the ordinary state of a class nobody
    /// has arranged yet, and deliberately not an error. Every other failure is
    /// one: an unreadable file, TOML that does not parse, and a profile that no
    /// longer validates each come back typed, for a caller whose whole answer
    /// to them is a warning and the bridge it already has.
    pub fn load(&self, class: &ShipClassKey) -> Result<Option<ValidatedProfile>, LayoutStoreError> {
        let path = self.path_for(class);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(LayoutStoreError::Read {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                })
            }
        };
        let profile =
            BridgeProfile::from_toml(&text).map_err(|source| LayoutStoreError::Profile {
                path: path.display().to_string(),
                source,
            })?;
        let validated = profile
            .validate()
            .map_err(|source| LayoutStoreError::Profile {
                path: path.display().to_string(),
                source,
            })?;
        Ok(Some(validated))
    }
}

/// Every authored surface `layout` is carrying, across all its monitors.
///
/// Composed from [`BridgeLayout::reserved_on`] rather than added to the law as a
/// method of its own: the law has no interest in the whole-bridge list, and this
/// is the only place that asks the question.
fn reservations(layout: &BridgeLayout) -> Vec<String> {
    layout
        .monitors()
        .iter()
        .flat_map(|m| layout.reserved_on(m))
        .collect()
}

/// Write `contents` to `path` so that a reader never sees a partial file.
///
/// Temp-then-rename: the bytes go to a sibling temporary, are flushed to the
/// device with `sync_all`, and the temporary is renamed over the target. A
/// process killed at any point leaves either the previous file intact or the new
/// one complete — the failure mode this avoids is a host crashing mid-save and
/// the *next* boot finding half a TOML file, which would read as "your saved
/// layout is corrupt" for a bridge that was fine.
///
/// # Windows
///
/// `std::fs::rename` is `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, so
/// unlike a bare `MoveFile` it does replace an existing destination, and within
/// one volume it is the closest thing Windows offers to a POSIX `rename`. Two
/// consequences are worth stating rather than discovering:
///
///  * It can fail with a sharing violation while another process holds the
///    destination open — an editor, an indexer, a virus scanner. That surfaces
///    as an ordinary [`LayoutStoreError::Write`]; the **previous file is
///    untouched**, which is the whole point, and the next accepted change tries
///    again.
///  * The temporary carries this process's id, so two hosts saving the same
///    class at the same moment cannot write each other's temporary. Last writer
///    wins on the target, which is the right answer for a per-user setting.
///
/// The temporary is removed on a failed rename, so a store that has been failing
/// to save does not silently fill with debris.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "layout".to_string());
    let temp = dir.join(format!("{stem}.{}.tmp", std::process::id()));

    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

// ── failures ────────────────────────────────────────────────────────────────

/// Why a saved layout could not be written or read.
///
/// Every variant names the file, because the operator's next move is to look at
/// it — and because the *only* thing a caller does with one of these is put it
/// in a warning and carry on with the bridge it already has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutStoreError {
    /// The layout carries authored `--pane` surfaces, which
    /// [`BridgeLayout::to_validated_profile`] does not re-emit — so writing it
    /// would drop them. See the
    /// [module note](self#what-may-be-saved-lobby-built-layouts-and-only-those).
    WouldDropReservations { class: String, labels: Vec<String> },
    /// The file could not be written (a full disk, a read-only directory, a
    /// destination another process is holding open).
    Write { path: String, detail: String },
    /// The file is there but could not be read.
    Read { path: String, detail: String },
    /// The file's contents are not a bridge profile this build accepts — it did
    /// not parse, or it parsed and did not validate.
    Profile { path: String, source: ProfileError },
}

impl std::fmt::Display for LayoutStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutStoreError::WouldDropReservations { class, labels } => write!(
                f,
                "the bridge layout for ship class {class:?} carries {} console(s) an authored \
                 --profile opened ({}), and a saved layout does not record those — writing it \
                 would drop them from the file and leave the next boot reading those screens as \
                 free. A saved layout is only ever one the lobby built",
                labels.len(),
                labels
                    .iter()
                    .map(|l| format!("{l:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            LayoutStoreError::Write { path, detail } => {
                write!(
                    f,
                    "saved bridge layout {path} could not be written: {detail}"
                )
            }
            LayoutStoreError::Read { path, detail } => {
                write!(f, "saved bridge layout {path} could not be read: {detail}")
            }
            LayoutStoreError::Profile { path, source } => {
                write!(f, "saved bridge layout {path} is not usable: {source}")
            }
        }
    }
}

impl std::error::Error for LayoutStoreError {}

#[cfg(test)]
#[path = "layout_store_tests.rs"]
mod tests;
