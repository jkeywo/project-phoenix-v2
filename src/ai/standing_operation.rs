//! Ownership decisions shared by standing Tractor, Dock and Umbilical orders.

#[derive(Clone, Copy)]
pub(crate) struct OperationFacts {
    pub order_present: bool,
    pub ready: bool,
    pub active: bool,
    pub owned: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OperationDecision {
    pub start: bool,
    pub claim: bool,
    pub withdraw: bool,
}

pub(crate) fn decide(facts: OperationFacts) -> OperationDecision {
    let start = facts.order_present && facts.ready && !facts.active;
    OperationDecision {
        start,
        claim: facts.order_present && (facts.active || start) && !facts.owned,
        withdraw: !facts.order_present && facts.active && facts.owned,
    }
}

#[cfg(test)]
mod tests {
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
}
