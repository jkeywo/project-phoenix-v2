use super::*;

#[test]
fn standing_orders_wait_start_adopt_and_protect_manual_operations() {
    // Order, readiness, physical activity, ownership -> start, claim, withdraw.
    for (facts, expected) in [
        ((false, false, false, false), (false, false, false)),
        ((false, true, false, false), (false, false, false)),
        ((false, false, true, false), (false, false, false)),
        ((false, true, true, false), (false, false, false)),
        ((false, false, false, true), (false, false, false)),
        ((false, true, false, true), (false, false, false)),
        ((false, false, true, true), (false, false, true)),
        ((false, true, true, true), (false, false, true)),
        ((true, false, false, false), (false, false, false)),
        ((true, true, false, false), (true, true, false)),
        ((true, false, true, false), (false, true, false)),
        ((true, true, true, false), (false, true, false)),
        ((true, false, false, true), (false, false, false)),
        ((true, true, false, true), (true, false, false)),
        ((true, false, true, true), (false, false, false)),
        ((true, true, true, true), (false, false, false)),
    ] {
        let decision = decide(OperationFacts {
            order_present: facts.0,
            ready: facts.1,
            active: facts.2,
            owned: facts.3,
        });
        assert_eq!(
            (decision.start, decision.claim, decision.withdraw),
            expected
        );
    }
}
