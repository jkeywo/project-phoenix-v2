//! The delayed weighted Backfill response picker (issue #1343) — pure Rust, no
//! Bevy, no clock, no RNG resource (AGENTS.md rule 10).
//!
//! A dialogue node's responses may each author two numbers,
//! [`CommsResponseAi::weight`] and [`CommsResponseAi::delay_seconds`]. Together
//! they say what an *unmanned* Comms console does with a live conversation: wait
//! the authored number of simulation seconds, then pick among the responses that
//! carry a positive weight, in proportion to those weights.
//!
//! # Why this exists beside the authored response policy
//!
//! The `comms_respond` [`AiPolicy`](crate::ai::policy::AiPolicy) channel every
//! shipped hull authors resolves to `respond_to_message` with
//! `response_index = 0` — the FIRST response, always. That is a sane fallback
//! for a hail acknowledgement and exactly wrong for a decision: Falling Skyway's
//! lift conversation puts "stand by" at index 0 (it has to; it is the option a
//! human reaches for while they think), so a backfilled console answering by
//! index would stall all three claims for ever and the act would never resolve.
//!
//! The two mechanisms do not compete for the same node.
//! [`node_authors_ai_choice`] is the switch: a node whose responses author no
//! weight at all is decided by the legacy policy exactly as it was, and a node
//! that authors one is decided here and the policy is not consulted for it.
//!
//! # What is deliberately NOT here
//!
//! No "the AI may only pick safe options" category, and no Captain-only
//! decision surface. A weighted pick is submitted through the ordinary
//! `RespondToMessage` admission path a human's press goes through, and every
//! guard downstream of admission — the router's bounds check, its
//! `active_dialogues` staleness check, its range gate, and the scenario's own
//! re-checks inside `on_pick` — applies identically to both. The weights say
//! what an absent officer would plausibly do, not what the engine will allow.

use crate::comms::content::CommsResponse;

/// One selectable entry of a node's weighted pool: the response index a
/// `RespondToMessage` submits, and the authored weight it carries.
///
/// Indices are the SHOWN indices, not pool positions — the pool skips
/// zero-weight and unavailable responses, so the two disagree the moment an
/// author puts a stand-by at index 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeightedResponse {
    /// Index into the node's `responses`, i.e. what the wire command carries.
    pub index: usize,
    /// The authored weight, always `> 0` (a zero-weight response never enters
    /// the pool at all).
    pub weight: u32,
}

/// Whether this node's responses carry Backfill choice metadata at all.
///
/// The switch between the two mechanisms, and it keys on **weight** rather than
/// on the delay: a delay with nothing to choose between says nothing, whereas a
/// weight is a complete instruction on its own (`ai_delay_seconds` absent is a
/// zero-second wait). A node with no authored weight anywhere keeps the legacy
/// first-response policy, which is every conversation in every world but
/// Falling Skyway's lift band.
pub fn node_authors_ai_choice(responses: &[CommsResponse]) -> bool {
    responses.iter().any(|r| r.ai.weight.is_some())
}

/// How long an unmanned console sits on this node before answering, in whole
/// simulation seconds.
///
/// The LONGEST delay any response on the node authors, because the wait happens
/// before the choice is made and so cannot be a property of the response that
/// wins. Falling Skyway authors the same five seconds on all three of a lift
/// node's options, which is the shape this rule is for; the maximum is the
/// answer that respects every authored pause rather than whichever one the
/// author happened to write first.
///
/// Zero when nothing authors a delay — the pick then lands on the first
/// evaluation tick that sees the node.
pub fn choice_delay_seconds(responses: &[CommsResponse]) -> u32 {
    responses
        .iter()
        .filter_map(|r| r.ai.delay_seconds)
        .max()
        .unwrap_or(0)
}

/// The responses an unmanned console may actually pick, in shown order.
///
/// Two exclusions, both from the acceptance criteria and both absolute:
///
///   * `ai_weight = 0`, or no authored weight at all, FORBIDS automatic
///     selection. That is how "stand by" stays off a backfilled console
///     (weight 0) and how Falling Skyway's confrontation options — which author
///     nothing — stay out of reach of one.
///   * an UNAVAILABLE response never enters the pool. Availability is the
///     message-wide sender-in-range flag the wire projection stamps
///     ([`response_views`](crate::comms::content::response_views)), the same
///     authoritative reachability the router itself gates on, so a pool built
///     while the sender is gone is empty rather than wrong.
///
/// An empty pool is a hold, not a fallback: the conversation stays open and
/// unanswered. Falling back to the legacy index-0 policy here would put the
/// backfilled console straight back on stand-by, which is the bug.
pub fn weighted_pool(responses: &[CommsResponse], available: bool) -> Vec<WeightedResponse> {
    if !available {
        return Vec::new();
    }
    responses
        .iter()
        .enumerate()
        .filter_map(|(index, r)| match r.ai.weight {
            Some(weight) if weight > 0 => Some(WeightedResponse { index, weight }),
            _ => None,
        })
        .collect()
}

/// The total weight of a pool — the exclusive upper bound the single RNG draw
/// is taken below.
///
/// SATURATING, for the same reason [`pick_by_draw`]'s walk is. Authored weights
/// are clamped only to `u32::MAX` apiece (`world::script::comms::take_count`),
/// so two large ones on one node overflow a plain sum: a debug build panics
/// inside the comms host and a release build wraps to a smaller total. Either is
/// unacceptable here. The panic is a dialogue bringing the server down, which
/// the module contract above forbids outright; the wrap is worse than the panic,
/// because it is a debug/release split in a number that feeds the draw, in a
/// simulation two peers have to agree on bit-for-bit.
///
/// Saturating makes an absurdly-weighted node merely absurd rather than fatal:
/// the total pins at `u32::MAX`, `pick_by_draw`'s cursor saturates at the same
/// place, and every draw lands on the first response whose running total reaches
/// it. Total, deterministic, and identical in both profiles — which is all this
/// has to be for a shape no mission authors.
pub fn pool_total_weight(pool: &[WeightedResponse]) -> u32 {
    pool.iter()
        .fold(0u32, |acc, entry| acc.saturating_add(entry.weight))
}

/// Turn one uniform draw in `0..pool_total_weight(pool)` into the response
/// index it selects.
///
/// The ordinary cumulative walk, in the pool's own (shown) order, so the mapping
/// from draw to answer is a pure function of the node and the draw — which is
/// what makes two peers on the same seed and the same state make the same
/// choice. `None` only for an empty pool or a draw at or past the total, both of
/// which the caller has already ruled out; it is a `None` rather than a panic
/// because a dialogue must never be able to bring the server down.
pub fn pick_by_draw(pool: &[WeightedResponse], draw: u32) -> Option<usize> {
    let mut cursor = 0u32;
    for entry in pool {
        cursor = cursor.saturating_add(entry.weight);
        if draw < cursor {
            return Some(entry.index);
        }
    }
    None
}

/// A fingerprint of everything about a node's responses that a pending choice
/// was armed against.
///
/// Stored beside the due tick so that REPLACING or REPRICING a node's options
/// cancels the wait that was running against the old ones: the fingerprint moves,
/// the host re-arms, and the fresh wait is sampled from the choices that are
/// actually on the screen. Availability folds in too, so a sender going out of
/// range and coming back is a new decision rather than a resumed one.
///
/// FNV-1a over the bytes, hand-rolled rather than `DefaultHasher`, for the
/// reason every other fold in this crate is hand-rolled: `DefaultHasher` is
/// explicitly not stable across releases, and this number is compared between
/// two peers' processes.
pub fn response_set_fingerprint(responses: &[CommsResponse], available: bool) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut acc = OFFSET;
    let fold_byte = |acc: &mut u64, byte: u8| {
        *acc ^= u64::from(byte);
        *acc = acc.wrapping_mul(PRIME);
    };
    let fold_u64 = |acc: &mut u64, value: u64| {
        for byte in value.to_le_bytes() {
            fold_byte(acc, byte);
        }
    };
    fold_u64(&mut acc, u64::from(available));
    fold_u64(&mut acc, responses.len() as u64);
    for r in responses {
        for byte in r.text.as_bytes() {
            fold_byte(&mut acc, *byte);
        }
        fold_u64(&mut acc, 0xff); // separator: "ab"+"c" must not fold as "a"+"bc"
        fold_u64(&mut acc, u64::from(r.important));
        match r.ai.weight {
            Some(w) => {
                fold_u64(&mut acc, 1);
                fold_u64(&mut acc, u64::from(w));
            }
            None => fold_u64(&mut acc, 0),
        }
        match r.ai.delay_seconds {
            Some(d) => {
                fold_u64(&mut acc, 1);
                fold_u64(&mut acc, u64::from(d));
            }
            None => fold_u64(&mut acc, 0),
        }
    }
    acc
}

#[cfg(test)]
mod tests {
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
}
