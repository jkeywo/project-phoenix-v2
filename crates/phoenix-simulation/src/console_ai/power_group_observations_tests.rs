use super::power_group_observations;
use phoenix_model::energy::{ConsumerEnergyReading, EnergyGroupReading, PowerEnergyBlackboard};

fn reference(energy: &PowerEnergyBlackboard) -> Vec<(&str, f32, f32)> {
    energy
        .groups
        .iter()
        .map(|group| {
            let (mut reserve, mut heat) = (1.0f32, 0.0f32);
            for consumer in &energy.consumers {
                let mut cursor = consumer.group.as_ref().map(|id| id.0.as_str());
                let mut belongs = false;
                for _ in 0..=energy.groups.len() {
                    let Some(id) = cursor else { break };
                    if id == group.id {
                        belongs = true;
                        break;
                    }
                    cursor = energy
                        .groups
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| item.parent.as_deref());
                }
                if belongs {
                    reserve = reserve.min(consumer.energy / consumer.capacity);
                    heat = heat.max(consumer.heat / consumer.trip_heat);
                }
            }
            (group.id.as_str(), reserve, heat)
        })
        .collect()
}

#[test]
fn grouped_facts_match_original_order_for_sparse_duplicate_and_cyclic_routes() {
    let mut energy = PowerEnergyBlackboard::default();
    energy.groups = [
        ("root", None),
        ("branch", Some("root")),
        ("leaf", Some("branch")),
        ("empty", None),
        ("broken", Some("missing")),
        ("cycle-a", Some("cycle-b")),
        ("cycle-b", Some("cycle-a")),
        ("branch", Some("empty")),
    ]
    .into_iter()
    .map(|(id, parent)| EnergyGroupReading {
        id: id.into(),
        parent: parent.map(str::to_string),
        ..Default::default()
    })
    .collect();
    for (index, group) in [
        Some("leaf"),
        Some("root"),
        None,
        Some("missing"),
        Some("broken"),
        Some("cycle-a"),
        Some("cycle-b"),
        Some("branch"),
    ]
    .into_iter()
    .enumerate()
    {
        energy.consumers.push(ConsumerEnergyReading {
            system: phoenix_model::identity::SystemId(format!("system-{index}")),
            group: group.map(|id| phoenix_model::identity::PowerGroupId(id.into())),
            energy: index as f32,
            capacity: 8.0,
            heat: (8 - index) as f32,
            trip_heat: 8.0,
            recover_heat: 1.0,
            thermal_stopped: false,
            intake_per_sec: 0.0,
            max_intake_per_sec: 0.0,
            flow_override_per_sec: None,
        });
    }
    for exceptional in [false, true] {
        if exceptional {
            energy.consumers[0].capacity = 0.0;
            energy.consumers[0].trip_heat = 0.0;
            energy.consumers[1].energy = -0.0;
        }
        for _ in 0..energy.consumers.len() {
            let actual = power_group_observations(&energy);
            let expected = reference(&energy);
            for (a, e) in actual.iter().zip(&expected) {
                assert_eq!(a.0, e.0);
                assert_eq!(a.1.to_bits(), e.1.to_bits());
                assert_eq!(a.2.to_bits(), e.2.to_bits());
            }
            energy.consumers.rotate_left(1);
        }
    }
    energy.consumers.clear();
    assert_eq!(power_group_observations(&energy), reference(&energy));
}
