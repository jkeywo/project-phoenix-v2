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
//! # Four properties, and each one is a rule rather than an implementation
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
//! 4. **The store enforces its own, narrower schema — at load, not only at
//!    save.** A saved layout records the viewscreen and the seated station
//!    consoles and *nothing else*. Being a valid `--profile` is not enough to
//!    get in here, because a `--profile` is a strictly larger language and the
//!    part of it this store cannot write is the part that does harm.
//!
//! # What may be saved: lobby-built layouts, and only those
//!
//! The store's schema is the **seats**: a `[[display]]` list in which every
//! `[[display.pane]]` names a `station`, and no `[[touch]]` or `[[media]]`
//! tables. Both doors enforce it, and they are the same rule facing opposite
//! ways:
//!
//!  * [`save`](LayoutStore::save) refuses
//!    [`LayoutStoreError::WouldDropReservations`] for a layout carrying
//!    reservations — the authored `--pane` surfaces a `--profile` opened, which
//!    [`BridgeLayout::reserved_on`] records and
//!    [`BridgeLayout::to_validated_profile`] does not re-emit.
//!  * [`load`](LayoutStore::load) refuses
//!    [`LayoutStoreError::NotALobbyLayout`] for a *file* carrying a station-less
//!    pane slot, a `[[touch]]` mapping or a `[[media]]` assignment.
//!
//! **Why the load side is not redundant.** A saved file *is* a bridge profile —
//! that is a feature, and the directory is one an operator browses — which is a
//! standing invitation to drop a `--profile` into it and see what happens.
//! Without the load-side refusal, what happens is a chain: the pre-apply adopts
//! the file, so `reserved` is populated on a run **nobody authored** (falsifying
//! the invariant this module states three times); the bridge acquires a phantom
//! occupant — a monitor reported full at a seat the operator cannot see, a
//! Station window with a permanently dead half, refusals naming a console that
//! is nowhere on screen; and the operator's first lobby press is then refused by
//! `save`'s `WouldDropReservations`, for ever, because the file that caused it is
//! never rewritten. One class, silently un-saveable, with the only warning in
//! the log being the wrong sentence. Refusing the file at the door makes that
//! whole sequence unreachable and hands the operator the moves that fix it.
//!
//! **What "refused" costs, stated honestly.** The file is left where it is —
//! an operator who hand-edited it wants to see what they wrote — but it is
//! ignored for this run, and their first press from the lobby files a saved
//! layout over it. So a refused file costs one session's arrangement rather
//! than the class, and the warning is the one chance to act on it before the
//! press. That is why the sentence carries the remedy rather than only the
//! complaint.
//!
//! # `[[touch]]` and `[[media]]`: refused, not preserved ([ai] decision)
//!
//! [`save`](LayoutStore::save) writes [`BridgeLayout::to_profile`] — an *empty*
//! profile with the displays written into it — so any `[[touch]]` or `[[media]]`
//! table an operator added to a store file is dropped by the next press.
//! [`BridgeLayout::write_displays_into`] exists precisely so a layout can be
//! written back *over* an existing profile, and preserving them that way was the
//! alternative. It is not taken, for two reasons rather than one:
//!
//!  * **It would not actually be lossless.** `write_displays_into` replaces the
//!    whole `[[display]]` list, so the file's `[[display.pane]]` participant
//!    slots would still be dropped — the silent loss would survive, one table
//!    over, and arrive through the door the reservation guard cannot see.
//!  * It puts a read, a parse and a policy for an unparseable file onto the
//!    **write** path, which runs on every accepted press.
//!
//! Refusing them at load does not *save* those tables: the file is ignored for
//! the run and the operator's first press files a saved layout over it, so they
//! are gone by the end of the session either way. What changes is that the loss
//! is announced before it happens, by a sentence naming where those tables
//! belong, rather than discovered afterwards by an operator wondering why their
//! touchscreen stopped working. What survives is the claim that is true in the
//! direction that matters: a file this store **wrote** is a profile an operator
//! may hand straight back to `--profile`. A `--profile` is not, in general, a
//! file this store will **read**.
//!
//! Those refusals are this module's half of a data-loss guard whose other half is
//! the *policy* in [`super::layout_store_systems`]: a `--profile` run never
//! writes at all. Either alone would do it today; both are here because the
//! failure they prevent is silent and permanent. Writing a `--profile`-seeded
//! layout would drop the operator's `[[display.pane]]` participant slots out of
//! the file, and the *next* boot would read those screens as free and move the
//! viewscreen on top of a crew member's live console — the exact failure
//! `reserved` exists to prevent, re-introduced by the save. A layout the lobby
//! built is reservation-free by construction (nothing but
//! [`BridgeLayout::adopt_profile`] ever populates `reserved`, and — *because*
//! [`load`](LayoutStore::load) refuses a station-less pane slot — nothing but an
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

/// The extension [`write_atomically`]'s in-flight temporary carries, at the end
/// of the `<file name>.<process id>.tmp` shape
/// [`LayoutStore::sweep_temporaries`] clears. Named once so the writer and the
/// sweeper cannot drift apart and leave debris nothing collects.
pub const TEMP_EXTENSION: &str = "tmp";

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
///
/// [ai] One Windows caveat, recorded rather than fixed: a hull template named
/// for a DOS device — `con.toml`, `nul.toml`, `aux.toml`, `prn.toml`,
/// `com1.toml` … — reduces to that reserved word, and Windows resolves
/// `…\bridge-layouts\con.toml` to the console device rather than to a file. That
/// class's saves therefore fail with an ordinary [`LayoutStoreError::Write`]
/// warning and it simply never remembers its bridge, which is the same
/// degradation an unwritable directory gets. No shipped hull is named that, the
/// failure is loud and costs nothing else, and renaming the template fixes it —
/// so the key deliberately does **not** grow a reserved-name escape that would
/// make the file name stop matching the hull the operator is looking at.
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
    ///
    /// `to_profile` starts from an **empty** profile, so this overwrites rather
    /// than merges: whatever the file held before is gone. That is safe only
    /// because [`load`](Self::load) refuses everything a merge would have had to
    /// preserve, which is the pair explained in the
    /// [module note](self#touch-and-media-refused-not-preserved-ai-decision).
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
    ///
    /// # Two doors, outermost first
    ///
    /// The file is put through [`BridgeProfile::validate`] — every refusal a
    /// hand-authored `--profile` meets — and *then* through this store's own,
    /// narrower schema, which refuses [`LayoutStoreError::NotALobbyLayout`] for
    /// a station-less `[[display.pane]]`, a `[[touch]]` mapping or a `[[media]]`
    /// assignment. That order is deliberate: "this is not a bridge profile" is
    /// the more fundamental complaint and should be the one an operator is told
    /// first, and it keeps every existing refusal reading exactly as it did.
    ///
    /// The second door is what makes the invariant `save` relies on true rather
    /// than hoped for — see the
    /// [module note](self#what-may-be-saved-lobby-built-layouts-and-only-those)
    /// for the phantom-occupant chain it cuts.
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
        let found = beyond_the_stores_schema(&profile);
        if !found.is_empty() {
            return Err(LayoutStoreError::NotALobbyLayout {
                path: path.display().to_string(),
                found,
            });
        }
        Ok(Some(validated))
    }

    /// Remove the `*.tmp` siblings a hard-killed host left in this directory,
    /// answering what was removed.
    ///
    /// [`write_atomically`] removes its own temporary on either failure, so an
    /// *ordinary* failed save leaves nothing behind. This is for the case
    /// nothing runs to clean up after: a power cut, a taskbar close or a
    /// `SIGKILL` landing between the create and the rename. The directory is one
    /// an operator browses, and `alliance_destroyer.toml.5732.tmp` beside their
    /// layouts is debris they have to reason about.
    ///
    /// A directory that is not there, and a file that will not delete, are both
    /// silently nothing: this is tidying, and nothing depends on it having
    /// worked.
    ///
    /// What is swept is the **writer's own name shape** —
    /// `<file name>.<process id>.tmp`, [`names_a_temporary`] — and not every
    /// `*.tmp` in the directory. This is a directory an operator opens, so their
    /// own `notes.tmp` beside their layouts is a file this has no business
    /// deleting; matching on the extension alone would take it on the next
    /// launch, silently, with nothing but a debug line to say so.
    ///
    /// [ai] Swept by name rather than by age, and the race that buys is
    /// recorded rather than closed: a second host of the same user launching in
    /// the microseconds another is between its create and its rename would
    /// delete that temporary. The victim reports an ordinary
    /// [`LayoutStoreError::Write`] with its **previous file untouched** and
    /// files the arrangement again on the next accepted change — which is the
    /// failure mode a sharing violation already has. An age threshold would need
    /// a clock and a number, to close a window that is already a documented
    /// no-op.
    pub fn sweep_temporaries(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut swept = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let is_debris = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(names_a_temporary);
            if is_debris && std::fs::remove_file(&path).is_ok() {
                swept.push(path);
            }
        }
        swept.sort();
        swept
    }
}

/// Everything in `profile` that a **saved layout** does not record, named the
/// way it appears in the file so the operator can go and find it.
///
/// Empty for a file the lobby could have written, which is what
/// [`LayoutStore::load`] requires. See the
/// [module note](self#what-may-be-saved-lobby-built-layouts-and-only-those).
fn beyond_the_stores_schema(profile: &BridgeProfile) -> Vec<String> {
    // DESTRUCTURED, so that a field added to `BridgeProfile` is a BUILD FAILURE
    // here rather than a silent wipe. This function is the whole of the store's
    // schema: anything it does not name is a table `save` overwrites out of the
    // file on the next press (`to_profile` starts from an empty profile), so a
    // door that hand-walked the fields would go on accepting the one thing
    // nobody had thought about yet. `version` is the profile's own and is
    // checked by `validate` a few lines up the call.
    let BridgeProfile {
        version: _,
        displays,
        touch,
        media,
    } = profile;
    let mut found = Vec::new();
    for display in displays {
        for pane in &display.panes {
            // The `--pane <NAME>` participant shape (issue #1122): a pane that
            // belongs to no station. `BridgeLayout::to_validated_profile` never
            // emits one, so a file holding one was not written here.
            if pane.station.is_none() {
                found.push(format!(
                    "a {PANE_TABLE} on {:?} with no `station =` ({:?})",
                    display.id, pane.label
                ));
            }
        }
    }
    for touch in touch {
        found.push(format!("a {TOUCH_TABLE} mapping ({:?})", touch.device));
    }
    for media in media {
        found.push(format!("a {MEDIA_TABLE} assignment ({:?})", media.surface));
    }
    found
}

/// The three table names [`beyond_the_stores_schema`] can report, spelled the
/// way they appear in the file.
///
/// Named once because they are used twice: to *say what was found*, and — by
/// [`LayoutStoreError::NotALobbyLayout`]'s remedy — to work out **which
/// imperative to give the operator**. A remedy assembled from a different
/// spelling than the finding would eventually tell somebody to delete a table
/// their file does not contain, which is the defect this pair exists to close.
const PANE_TABLE: &str = "[[display.pane]]";
/// See [`PANE_TABLE`].
const TOUCH_TABLE: &str = "[[touch]]";
/// See [`PANE_TABLE`].
const MEDIA_TABLE: &str = "[[media]]";

/// The moves that fix `found`, composed from what is actually **in** it.
///
/// One blended sentence naming every refusal class was the alternative, and it
/// is wrong in the ordinary case rather than in an exotic one: an operator whose
/// file carries a `[[touch]]` mapping and nothing else was told to "delete the
/// `[[display.pane]]` entries that have no `station =`" — entries their file
/// does not contain — and was never told what to do with the table that was
/// actually refused. So each clause appears only when its own class was found,
/// and the one move that always works (`--profile`) is the tail.
fn remedy_for(found: &[String]) -> String {
    let present = |table: &str| found.iter().any(|f| f.contains(table));
    let mut clauses: Vec<String> = Vec::new();
    if present(PANE_TABLE) {
        clauses.push(format!(
            "delete the {PANE_TABLE} entries that have no `station =`"
        ));
    }
    let tables: Vec<&str> = [TOUCH_TABLE, MEDIA_TABLE]
        .into_iter()
        .filter(|t| present(t))
        .collect();
    if !tables.is_empty() {
        clauses.push(format!(
            "move every {} table into a --profile of your own",
            tables.join(" and ")
        ));
    }
    // Always available, and the only one that keeps the file as it stands.
    clauses.push("point --profile at this file instead".to_string());

    let mut remedy = clauses.join(", or ");
    // It opens a sentence. Every clause above starts with an ASCII verb.
    if let Some(first) = remedy.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    remedy
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
/// The temporary is removed on either failure, so a store that has been failing
/// to save does not fill with debris. A **hard** kill between the create and the
/// rename does leave one, because nothing runs at all — that is what
/// [`LayoutStore::sweep_temporaries`] is for, and why the claim above is about
/// an ordinary failure rather than about every one.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "layout".to_string());
    // `<file name>.<process id>.tmp`. `names_a_temporary` is the reader of this
    // shape and the two are a pair — see [`LayoutStore::sweep_temporaries`].
    let temp = dir.join(format!("{stem}.{}.{TEMP_EXTENSION}", std::process::id()));

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

/// Whether `name` is a temporary [`write_atomically`] left behind:
/// `<file name>.<process id>.`[`TEMP_EXTENSION`].
///
/// The pid component is required, and required to be digits, so that this
/// matches the writer's own output and nothing else. An operator's `notes.tmp`
/// or `old.toml.tmp` in the same directory is not debris this wrote and is not
/// debris this deletes — see [`LayoutStore::sweep_temporaries`]. The pid is not
/// checked against a *live* process: the whole point is that the host which
/// wrote it is gone, and a pid is reused.
fn names_a_temporary(name: &str) -> bool {
    let Some(rest) = name.strip_suffix(&format!(".{TEMP_EXTENSION}")) else {
        return false;
    };
    let Some((stem, pid)) = rest.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty() && !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit())
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
    /// The file parses and validates as a bridge profile, but it is not one the
    /// lobby wrote: it carries content the store's own narrower schema does not
    /// hold — a station-less `[[display.pane]]`, a `[[touch]]` mapping or a
    /// `[[media]]` assignment. The mirror of
    /// [`WouldDropReservations`](Self::WouldDropReservations) at the other door;
    /// see the
    /// [module note](self#what-may-be-saved-lobby-built-layouts-and-only-those).
    NotALobbyLayout { path: String, found: Vec<String> },
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
            LayoutStoreError::NotALobbyLayout { path, found } => write!(
                f,
                "saved bridge layout {path} is not one the lobby wrote — it carries {}. {}: a \
                 saved layout records the viewscreen and the seated station consoles and \
                 nothing else",
                found.join(", "),
                remedy_for(found)
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
