//! What is on the mod-pack shelf (issue #1366, PRD #1355).
//!
//! # The problem this exists to answer, and the one it deliberately does not
//!
//! A browser host loads a mod pack from a `<input type="file">`. The native
//! host's lobby document does not have one — [`super::host_lobby::document`]
//! strips `#mod-pack-upload` out, because a file input with no page-lifetime
//! handler behind it is a control that silently does nothing — and this process
//! has no file dialog to open instead. Taking one would mean taking a
//! dependency (a native picker crate), and PRD #855's whole delivery story is a
//! single self-contained binary.
//!
//! So the native answer is a **shelf**: the operator names a directory at the
//! prompt, the host scans it, and the landing screen offers what is there. This
//! module is the whole of "what is on the shelf" and nothing else:
//!
//! * it does **not** read, validate or install a pack. That is the existing
//!   path, unchanged — [`crate::world::mod_pack::validate_mod_pack`] judges the
//!   archive and [`crate::entities::config_cache::push_mod_pack`] installs it —
//!   and [`super::host_lobby::packs`] is the thin adapter between the two;
//! * it does **not** touch the filesystem. [`shelf_from_listing`] takes a
//!   DIRECTORY LISTING, which is what makes every rule below testable under an
//!   ordinary `cargo test` with no temp directory, no permissions and no
//!   platform in it. The one function here that does touch a disk is
//!   [`read_shelf_listing`], and it does nothing but turn `read_dir` into that
//!   listing — there is no decision in it to test.
//!
//! # The listing is the seam, and the id is the gate
//!
//! The surface sends back a **file name it was offered**, never a path. What
//! comes back off a bridge is untrusted the moment it is a string the host
//! joins onto a directory, so nothing here ever joins one: [`offered`] looks the
//! name up in the shelf this host itself produced, and a name that is not on the
//! shelf is `None`. A `..\..\Windows\System32` cannot be on the shelf, because
//! [`shelf_from_listing`] refuses to put a name with a separator or a traversal
//! segment on it in the first place — belt and braces, in that order.

use std::path::Path;

/// The archive extension a mod pack is published as.
///
/// The store-only ZIP `editor/mod-pack-export.js` writes and
/// [`crate::world::mod_pack::read_store_zip`] reads. Compared case-insensitively
/// below, because Windows hands back whatever case the file was created with and
/// an operator who saved `PACK.ZIP` has still put a pack on the shelf.
pub const PACK_EXTENSION: &str = ".zip";

/// One entry of a directory listing, as this module needs to see it.
///
/// Deliberately not `std::fs::DirEntry`: that type cannot be constructed by a
/// test, which is exactly the property that would push the rules below onto a
/// real filesystem. Two fields, because two are all any rule here reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfListingEntry {
    /// The entry's own name within the directory — never a path.
    pub name: String,
    /// Whether it is a regular file. A directory called `pack.zip` is not a
    /// pack, and neither is anything else that cannot be read as bytes.
    pub is_file: bool,
}

impl ShelfListingEntry {
    /// A regular file called `name`. The shape every test builds.
    pub fn file(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_file: true,
        }
    }

    /// A subdirectory called `name`.
    pub fn dir(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_file: false,
        }
    }
}

/// One pack on offer.
///
/// `file` is both the archive's name in the scanned directory and the id the
/// surface sends back to ask for it — one token, so nothing downstream keeps a
/// mapping table that could disagree with the shelf it was built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfPack {
    /// The archive's file name, exactly as the directory reported it.
    pub file: String,
    /// What the landing shows: the file name without its `.zip`.
    ///
    /// Not the pack manifest's `[pack] name`, and that is a decision rather than
    /// a shortcut: reading the name out would mean opening and parsing every
    /// archive in the directory just to draw a list, and a malformed one would
    /// then have to be either hidden (a pack the operator can see in Explorer
    /// but not on the shelf) or shown with an error nobody asked for yet. The
    /// pack's own identity arrives when it is installed, from the validator that
    /// was going to read it anyway.
    pub label: String,
}

/// The packs a directory listing offers, in the order they are shown.
///
/// The whole of the shelf rule, and pure so that each clause below is a test
/// rather than a claim about `read_dir`:
///
///  * **files only.** A directory named `something.zip` is not an archive.
///  * **`.zip` only**, case-insensitively, and never the bare extension — a file
///    actually called `.zip` has no name to show.
///  * **a plain file name only.** Anything carrying `/`, `\` or a `..` segment
///    is refused rather than sanitised: a listing should never contain one, so
///    one that does is a surprise, and the safe answer to a surprise on a path
///    that will be opened is to drop it.
///  * **sorted**, case-insensitively and then by bytes, so two hosts scanning
///    the same folder offer the same list in the same order whatever the
///    filesystem felt like returning.
///  * **deduplicated by name**, because a listing is a set and a repeated name
///    would be two rows pointing at one file.
pub fn shelf_from_listing(entries: &[ShelfListingEntry]) -> Vec<ShelfPack> {
    let mut packs: Vec<ShelfPack> = entries
        .iter()
        .filter(|entry| entry.is_file)
        .filter(|entry| is_pack_file_name(&entry.name))
        .map(|entry| ShelfPack {
            file: entry.name.clone(),
            label: label_for(&entry.name),
        })
        .collect();
    packs.sort_by(|a, b| {
        a.file
            .to_lowercase()
            .cmp(&b.file.to_lowercase())
            .then_with(|| a.file.cmp(&b.file))
    });
    packs.dedup_by(|a, b| a.file == b.file);
    packs
}

/// The pack on `shelf` the surface named, or `None`.
///
/// The gate every install goes through. The surface's `file` is untrusted input
/// — it crosses a bridge and this host is about to open it — so it is never
/// joined onto the scanned directory; it is looked up in the shelf THIS host
/// produced, and a name that is not there is refused. That makes "the operator
/// asked for a pack that has since been deleted" and "the page asked for
/// something it was never offered" the same answer, which is the right answer to
/// both.
pub fn offered<'a>(shelf: &'a [ShelfPack], file: &str) -> Option<&'a ShelfPack> {
    shelf.iter().find(|pack| pack.file == file)
}

/// Whether `name` is a plain file name naming a `.zip`.
fn is_pack_file_name(name: &str) -> bool {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return false;
    }
    if name == ".." || name == "." {
        return false;
    }
    let lowered = name.to_lowercase();
    lowered.ends_with(PACK_EXTENSION) && lowered.len() > PACK_EXTENSION.len()
}

/// The shown name: the file name with its extension taken off.
fn label_for(name: &str) -> String {
    name[..name.len() - PACK_EXTENSION.len()].to_string()
}

/// Read one directory into a [`ShelfListingEntry`] list.
///
/// The ONE impure function in this module, and it is deliberately decision-free:
/// everything that could be got wrong lives in [`shelf_from_listing`] above,
/// which never sees a filesystem. An unreadable entry is skipped rather than
/// failing the scan — a folder holding one file the process cannot stat still
/// has a shelf — but an unreadable DIRECTORY is an error, because an operator
/// who named a folder at the prompt has to be told the host could not open it.
pub fn read_shelf_listing(dir: &Path) -> Result<Vec<ShelfListingEntry>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut entries = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        entries.push(ShelfListingEntry { name, is_file });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_of_archives_becomes_the_shelf_in_a_stable_order() {
        // The ordinary case, and the reason the whole module takes a listing:
        // this is the entire feature and there is no directory anywhere in it.
        let shelf = shelf_from_listing(&[
            ShelfListingEntry::file("zulu.zip"),
            ShelfListingEntry::file("Alpha.zip"),
            ShelfListingEntry::file("mike.zip"),
        ]);
        assert_eq!(
            shelf.iter().map(|p| p.file.as_str()).collect::<Vec<_>>(),
            ["Alpha.zip", "mike.zip", "zulu.zip"],
            "case-insensitive, so two hosts scanning one folder offer one order"
        );
        assert_eq!(shelf[0].label, "Alpha");
    }

    #[test]
    fn everything_that_is_not_a_pack_archive_stays_off_the_shelf() {
        let shelf = shelf_from_listing(&[
            // A directory named like an archive is not one.
            ShelfListingEntry::dir("tempting.zip"),
            // Neither is a file with any other extension…
            ShelfListingEntry::file("readme.txt"),
            ShelfListingEntry::file("scenarios.toml"),
            // …nor a file whose whole name IS the extension: there would be
            // nothing to show in the row.
            ShelfListingEntry::file(".zip"),
            // The one that is.
            ShelfListingEntry::file("good.zip"),
        ]);
        assert_eq!(
            shelf.iter().map(|p| p.file.as_str()).collect::<Vec<_>>(),
            ["good.zip"]
        );
    }

    #[test]
    fn the_extension_is_matched_without_caring_about_case() {
        // Windows hands back the case the file was created with, and an operator
        // who saved PACK.ZIP has still put a pack on the shelf.
        let shelf = shelf_from_listing(&[
            ShelfListingEntry::file("SHOUTING.ZIP"),
            ShelfListingEntry::file("Mixed.Zip"),
        ]);
        assert_eq!(
            shelf.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(),
            ["Mixed", "SHOUTING"]
        );
    }

    #[test]
    fn a_name_that_is_a_path_never_reaches_the_shelf() {
        // A directory listing should never contain one of these, which is
        // exactly why one appearing is dropped rather than repaired: the name on
        // the shelf is the name this host will open, and the only safe response
        // to a surprise in it is not to offer it. The lookup gate in `offered`
        // is the second half of the same answer.
        let shelf = shelf_from_listing(&[
            ShelfListingEntry::file("../escape.zip"),
            ShelfListingEntry::file("..\\escape.zip"),
            ShelfListingEntry::file("nested/pack.zip"),
            ShelfListingEntry::file(".."),
            ShelfListingEntry::file("honest.zip"),
        ]);
        assert_eq!(
            shelf.iter().map(|p| p.file.as_str()).collect::<Vec<_>>(),
            ["honest.zip"]
        );
    }

    #[test]
    fn a_repeated_name_is_one_row() {
        // A listing is a set. Two rows pointing at one file would let the same
        // pack be installed twice, which the overlay stack would then refuse as
        // a duplicate pack id — a confusing way to learn about a rendering bug.
        let shelf = shelf_from_listing(&[
            ShelfListingEntry::file("twice.zip"),
            ShelfListingEntry::file("twice.zip"),
        ]);
        assert_eq!(shelf.len(), 1);
    }

    #[test]
    fn an_empty_folder_has_an_empty_shelf_rather_than_no_shelf() {
        // "The folder is empty" is a state the landing has words for; it must
        // not be reachable only by the scan failing.
        assert_eq!(shelf_from_listing(&[]), Vec::new());
    }

    #[test]
    fn only_a_pack_this_host_offered_can_be_asked_for() {
        // The gate. What comes back off the bridge is a string, and this is the
        // one place it is turned into a file this process will open — by lookup,
        // never by joining it onto the scanned directory.
        let shelf = shelf_from_listing(&[ShelfListingEntry::file("thin-margin.zip")]);
        assert_eq!(
            offered(&shelf, "thin-margin.zip").map(|p| p.file.as_str()),
            Some("thin-margin.zip")
        );
        // Never offered, deleted since the scan, or invented by a page older or
        // newer than this binary — all one answer, which is the right answer to
        // all three.
        assert!(offered(&shelf, "never-offered.zip").is_none());
        assert!(offered(&shelf, "../../secrets.zip").is_none());
        assert!(offered(&shelf, "").is_none());
    }
}
