use super::*;
use strum::IntoEnumIterator;

/// `target()` is hand-written for const-ness; this is what stops it
/// drifting from the `strum` serialisation the spec parser accepts.
#[test]
fn target_matches_strum_serialisation() {
    for cat in LogCat::iter() {
        let via_strum: &'static str = cat.into();
        assert_eq!(cat.target(), via_strum, "target() drifted for {cat:?}");
    }
}
