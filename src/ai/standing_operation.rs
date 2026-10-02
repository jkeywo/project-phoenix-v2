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
#[path = "standing_operation_tests.rs"]
mod tests;
