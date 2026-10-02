use super::*;

/// A `Stacked` pair (full width, half height each) must stay stacked on
/// resize, not collapse into the naive side-by-side tiling — the bug
/// issue #1420 fixes: before this, ANY window with 2+ panes was tiled
/// `pw / count` wide regardless of what split the profile authored.
#[test]
fn seated_panes_keep_their_authored_split_instead_of_tiling() {
    let window = (1920, 1080);
    let seated = vec![Some((0, 0, 1920, 540)), Some((0, 540, 1920, 540))];
    let targets = resize_tile_targets(window, &seated);
    assert_eq!(
        targets,
        vec![((0, 0), (1920, 540)), ((0, 540), (1920, 540)),]
    );
}

/// A `SideBySide` pair keeps working exactly as before.
#[test]
fn seated_side_by_side_pair_is_unaffected() {
    let window = (1920, 1080);
    let seated = vec![Some((0, 0, 960, 1080)), Some((960, 0, 960, 1080))];
    let targets = resize_tile_targets(window, &seated);
    assert_eq!(
        targets,
        vec![((0, 0), (960, 1080)), ((960, 0), (960, 1080)),]
    );
}

/// The legacy `--pane`-without-`--profile` fallback: panes with no
/// Station slot still tile side by side across the window.
#[test]
fn unseated_panes_tile_side_by_side_as_before() {
    let window = (1920, 1080);
    let seated = vec![None, None];
    let targets = resize_tile_targets(window, &seated);
    assert_eq!(
        targets,
        vec![((0, 0), (960, 1080)), ((960, 0), (960, 1080)),]
    );
}

/// A mixed window — one pane seated on a live Station slot, one with
/// none — is not something the profile format produces today, but the
/// function should still do the sane thing: the seated pane keeps its
/// slot, and the lone unresolved pane gets the rest of the window rather
/// than being squeezed by the seated pane's share.
#[test]
fn mixed_seated_and_unresolved_panes_resolve_independently() {
    let window = (1920, 1080);
    let seated = vec![Some((0, 0, 960, 1080)), None];
    let targets = resize_tile_targets(window, &seated);
    assert_eq!(
        targets,
        vec![((0, 0), (960, 1080)), ((0, 0), (1920, 1080)),]
    );
}

/// A lone Station-seated pane on its window takes its slot's own rect,
/// not the full window — a Station can be mid-transition between one and
/// two occupants.
#[test]
fn lone_seated_pane_takes_its_slot_not_the_full_window() {
    let window = (1920, 1080);
    let seated = vec![Some((0, 0, 1920, 540))];
    let targets = resize_tile_targets(window, &seated);
    assert_eq!(targets, vec![((0, 0), (1920, 540))]);
}

/// A lone unseated pane (the legacy single-`--pane` case) still fills
/// the whole window, as before this change.
#[test]
fn lone_unresolved_pane_fills_the_window() {
    let window = (1920, 1080);
    let seated = vec![None];
    let targets = resize_tile_targets(window, &seated);
    assert_eq!(targets, vec![((0, 0), (1920, 1080))]);
}
