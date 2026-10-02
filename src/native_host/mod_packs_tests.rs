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
