//! Predicates mirror the four real consumer filters, applied to actual emitted commands.
use project_phoenix::core::messages::{
    AdmittedCommand, RepairTarget, SystemControlPayload as Payload, SystemId,
};
use serde_json::{json, Value};

pub fn owns(index: usize, command: &AdmittedCommand, arcs: &[SystemId]) -> bool {
    match index {
        0 => {
            command.target.0 == "power-reactor"
                && matches!(command.payload, Payload::SetPowerGroupAllocation { .. })
        }
        1 => {
            arcs.contains(&command.target)
                && matches!(command.payload, Payload::SetShieldArcFocus { .. })
        }
        2 => {
            command.target.0 == "navigation"
                && matches!(
                    command.payload,
                    Payload::SetNavigationWaypoint { .. } | Payload::ClearNavigationWaypoint
                )
        }
        3 => {
            command.target.0 == "repair"
                && matches!(command.payload, Payload::DispatchRepairTeam { ref target, .. } if !matches!(target, RepairTarget::External))
        }
        _ => unreachable!(),
    }
}
pub fn projected(
    index: usize,
    commands: &[AdmittedCommand],
    arcs: &[SystemId],
) -> Vec<AdmittedCommand> {
    if index == 1 {
        // Actual Shields consumer walks authored arcs outside each arc's request sequence.
        arcs.iter()
            .flat_map(|arc| {
                commands
                    .iter()
                    .filter(move |c| &c.target == arc && owns(index, c, arcs))
                    .cloned()
            })
            .collect()
    } else {
        commands
            .iter()
            .filter(|c| owns(index, c, arcs))
            .cloned()
            .collect()
    }
}
pub fn evidence(commands: &[AdmittedCommand]) -> Value {
    json!(commands
        .iter()
        .map(|c| json!({"target":c.target,"payload":c.payload,
        "response_token":c.response_token,"feedback_correlation":c.feedback_correlation}))
        .collect::<Vec<_>>())
}
fn interleavings(a: &[AdmittedCommand], b: &[AdmittedCommand]) -> Vec<Vec<AdmittedCommand>> {
    if a.is_empty() {
        return vec![b.to_vec()];
    }
    if b.is_empty() {
        return vec![a.to_vec()];
    }
    let mut result = Vec::new();
    for tail in interleavings(&a[1..], b) {
        let mut row = vec![a[0].clone()];
        row.extend(tail);
        result.push(row);
    }
    for tail in interleavings(a, &b[1..]) {
        let mut row = vec![b[0].clone()];
        row.extend(tail);
        result.push(row);
    }
    result
}
pub fn assert_actual_chunk_interleavings(
    prefix: &[AdmittedCommand],
    chunks: &[Vec<AdmittedCommand>; 4],
    arcs: &[SystemId],
) -> usize {
    assert!(prefix.len() >= 2);
    assert!(
        chunks.iter().all(|chunk| !chunk.is_empty()),
        "every real producer must contribute"
    );
    let mut checked = 0;
    for first in 0..4 {
        for second in first + 1..4 {
            let mut original = prefix.to_vec();
            original.extend_from_slice(&chunks[first]);
            original.extend_from_slice(&chunks[second]);
            for suffix in interleavings(&chunks[first], &chunks[second]) {
                let mut reordered = prefix.to_vec();
                reordered.extend(suffix);
                assert!(reordered.starts_with(prefix));
                for consumer in 0..4 {
                    assert_eq!(
                        projected(consumer, &original, arcs),
                        projected(consumer, &reordered, arcs),
                        "actual complete consumer subsequence"
                    );
                }
                let unrelated = |q: &[AdmittedCommand]| {
                    q.iter()
                        .filter(|c| !(0..4).any(|i| owns(i, c, arcs)))
                        .cloned()
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    unrelated(&reordered),
                    prefix,
                    "foreign admitted prefix preserved in order"
                );
                checked += 1;
            }
        }
    }
    let mut reversed = prefix.to_vec();
    reversed.reverse();
    assert_ne!(
        reversed, prefix,
        "prefix oracle distinguishes actual operands/order"
    );
    checked
}
