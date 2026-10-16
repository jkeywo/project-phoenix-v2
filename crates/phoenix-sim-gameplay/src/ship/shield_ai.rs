//! Closed per-arc controller bindings. Values come only from the owning Console.
use crate::ai::state_tree::{NumericBindings, SupportedBindings};
use crate::core::messages::SystemId;

pub fn bindings(targets: &[SystemId]) -> (SupportedBindings, NumericBindings) {
    let mut booleans = SupportedBindings {
        facts: [
            "preferred_focus",
            "preferred_protection",
            "raised",
            "thermal_stopped",
        ]
        .map(str::to_owned)
        .into(),
        channels: ["raised", "focus"].map(str::to_owned).into(),
    };
    let mut numbers = NumericBindings {
        facts: [
            "strength_fraction",
            "reserve_fraction",
            "heat_fraction",
            "recent_damage",
        ]
        .map(str::to_owned)
        .into(),
        channels: Default::default(),
    };
    for target in targets {
        for prefix in [
            "target-visible",
            "target-preferred",
            "target-ready",
            "recovering",
        ] {
            booleans.facts.insert(format!("{prefix}-{}", target.0));
        }
        booleans.channels.insert(format!("recover-{}", target.0));
        for prefix in ["target-strength", "target-reserve"] {
            numbers.facts.insert(format!("{prefix}-{}", target.0));
        }
    }
    (booleans, numbers)
}
