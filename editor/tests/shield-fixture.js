import { stringify } from 'smol-toml';
import { createEnergyModel, defaultEnergyConsumer, defaultEnergyGroup } from '../workshop-energy.js';
import { defaultShieldArc } from '../workshop-shields.js';

export function shieldSource() {
  const value = { infrastructure: { capacity: [{ id: 'coolant', amount: 100, ceiling: 100 }] },
    resource_services: { coolant: { kind: 'capacity', id: 'coolant' } }, power_groups: {} };
  value.energy = createEnergyModel(value);
  value.shield_arc = ['fore', 'aft'].map(id => ({ ...defaultShieldArc(), id, label: id }));
  for (const arc of value.shield_arc) {
    const id = `shield-arc-${arc.id}`, group = `energy-${id}`, consumer = defaultEnergyConsumer(id);
    consumer.continuous_cost_per_sec = 0; consumer.discrete_cost = 0;
    value.energy.consumer.push(consumer); value.energy.group.push(defaultEnergyGroup(group));
    value.power_groups[group] = { label: arc.id, default_level: 2, min_level: 0, max_level: 4 };
  }
  return '# unrelated top note\n' + stringify(value) + '\n# unrelated footer\n';
}
