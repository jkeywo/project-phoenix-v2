//! Closed Power tree interface, derived from this ship's immutable hardware.
use crate::ai::state_tree::{NumericBindings, NumericBounds};
use crate::entities::config::EnergyConfigSection;
use std::collections::BTreeMap;

pub fn power_numeric_bindings(config: &EnergyConfigSection) -> NumericBindings {
    let mut facts: std::collections::BTreeSet<String> = [
        "reactor_reserve_fraction",
        "reactor_heat_fraction",
        "output_actual",
    ]
    .map(str::to_owned)
    .into();
    let mut channels = BTreeMap::from([
        (
            "reactor-output".into(),
            NumericBounds {
                min: 0.0,
                max: config.reactor.max_output_per_sec as f64,
            },
        ),
        (
            "reactor-cooling".into(),
            NumericBounds {
                min: 0.0,
                max: config.reactor.max_cooling_per_sec as f64,
            },
        ),
    ]);
    for group in &config.groups {
        facts.insert(format!("reserve-{}", group.id.0));
        facts.insert(format!("heat-{}", group.id.0));
        channels.insert(
            format!("flow-{}", group.id.0),
            NumericBounds {
                min: 0.0,
                max: group.max_flow_per_sec as f64,
            },
        );
        channels.insert(
            format!("cooling-{}", group.id.0),
            NumericBounds {
                min: 0.0,
                max: group.max_cooling_per_sec as f64,
            },
        );
    }
    NumericBindings { facts, channels }
}
