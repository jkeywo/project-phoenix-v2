use super::*;
use crate::comms::content::CommsResponseAi;

/// `(text, weight, delay)` → a response, so the fixtures below read as the
/// authored table they stand for.
fn r(text: &str, weight: Option<u32>, delay: Option<u32>) -> CommsResponse {
    CommsResponse {
        text: text.into(),
        important: false,
        ai: CommsResponseAi {
            weight,
            delay_seconds: delay,
        },
    }
}

/// The Falling Skyway lift node, in the shape the world authors it:
/// stand-by first and forbidden, grant and deny equal behind it.
fn lift_node() -> Vec<CommsResponse> {
    vec![
        r("stand_by", Some(0), Some(5)),
        r("lift", Some(1), Some(5)),
        r("deny", Some(1), Some(5)),
    ]
}

#[test]
fn a_node_with_no_authored_weight_is_left_to_the_legacy_policy() {
    let plain = vec![r("Acknowledge", None, None), r("Decline", None, None)];
    assert!(!node_authors_ai_choice(&plain));
    // A delay on its own is not an instruction: there is nothing to choose.
    let delay_only = vec![r("Acknowledge", None, Some(5))];
    assert!(!node_authors_ai_choice(&delay_only));
    assert!(node_authors_ai_choice(&lift_node()));
}

#[test]
fn the_wait_is_the_longest_delay_any_response_authors() {
    assert_eq!(choice_delay_seconds(&lift_node()), 5);
    assert_eq!(choice_delay_seconds(&[]), 0);
    assert_eq!(
        choice_delay_seconds(&[r("a", Some(1), None), r("b", Some(1), Some(9))]),
        9
    );
}

/// AC1: zero-weight and unweighted responses never enter the pool.
#[test]
fn zero_weight_and_unweighted_responses_are_never_selectable() {
    let pool = weighted_pool(&lift_node(), true);
    assert_eq!(
        pool,
        vec![
            WeightedResponse {
                index: 1,
                weight: 1
            },
            WeightedResponse {
                index: 2,
                weight: 1
            },
        ],
        "stand-by is authored weight 0 and must not be reachable"
    );

    // Havelock's confrontation options author nothing at all, and the same
    // rule keeps them out.
    let with_confront = vec![
        r("stand_by", Some(0), Some(5)),
        r("lift", Some(1), Some(5)),
        r("deny", Some(1), Some(5)),
        r("confront", None, None),
    ];
    assert_eq!(
        weighted_pool(&with_confront, true)
            .iter()
            .map(|e| e.index)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

/// AC1: an unavailable response never enters the pool — and since
/// availability is message-wide, an out-of-range sender empties it.
#[test]
fn an_unavailable_node_has_no_pool_at_all() {
    assert!(weighted_pool(&lift_node(), false).is_empty());
}

/// The draw→index mapping is total over the pool's weight range and lands
/// on the shown indices, not the pool positions.
#[test]
fn the_draw_walks_the_pool_in_shown_order() {
    let pool = weighted_pool(&lift_node(), true);
    assert_eq!(pool_total_weight(&pool), 2);
    assert_eq!(pick_by_draw(&pool, 0), Some(1));
    assert_eq!(pick_by_draw(&pool, 1), Some(2));
    assert_eq!(
        pick_by_draw(&pool, 2),
        None,
        "past the total selects nothing"
    );
    assert_eq!(pick_by_draw(&[], 0), None);
}

/// Uneven weights get proportional slices of the draw range.
#[test]
fn heavier_responses_own_more_of_the_range() {
    let node = vec![
        r("never", Some(0), None),
        r("rare", Some(1), None),
        r("common", Some(3), None),
    ];
    let pool = weighted_pool(&node, true);
    assert_eq!(pool_total_weight(&pool), 4);
    assert_eq!(pick_by_draw(&pool, 0), Some(1));
    for draw in 1..4 {
        assert_eq!(pick_by_draw(&pool, draw), Some(2), "draw {draw}");
    }
}

/// Authored data must not be able to overflow the total — a panic here is a
/// dialogue taking the server down in a debug build, and a wrap is a
/// debug/release disagreement inside the draw of a deterministic sim.
#[test]
fn absurd_authored_weights_saturate_rather_than_overflow() {
    let node = vec![
        r("enormous", Some(u32::MAX), None),
        r("also_enormous", Some(u32::MAX), None),
    ];
    let pool = weighted_pool(&node, true);
    let total = pool_total_weight(&pool);
    assert_eq!(total, u32::MAX, "the total pins rather than wrapping");

    // And the pick stays total over the whole draw range: the first entry
    // already saturates the cursor, so every draw resolves to it and none
    // falls off the end.
    for draw in [0, 1, u32::MAX / 2, u32::MAX - 1] {
        assert_eq!(pick_by_draw(&pool, draw), Some(0), "draw {draw}");
    }
}

/// The fingerprint is what turns "the options changed under a running wait"
/// into a cancelled wait rather than an answer to a screen nobody is
/// looking at any more.
#[test]
fn the_fingerprint_moves_when_the_options_do() {
    let before = response_set_fingerprint(&lift_node(), true);
    assert_eq!(
        before,
        response_set_fingerprint(&lift_node(), true),
        "the same node must fingerprint the same, or every wait re-arms for ever"
    );

    // The repriced node: the lift is gone because the board moved.
    let repriced = vec![r("stand_by", Some(0), Some(5)), r("deny", Some(1), Some(5))];
    assert_ne!(before, response_set_fingerprint(&repriced, true));
    // A reweighted node, same texts.
    let reweighted = vec![
        r("stand_by", Some(0), Some(5)),
        r("lift", Some(3), Some(5)),
        r("deny", Some(1), Some(5)),
    ];
    assert_ne!(before, response_set_fingerprint(&reweighted, true));
    // And availability is part of the decision the wait was armed against.
    assert_ne!(before, response_set_fingerprint(&lift_node(), false));
}
