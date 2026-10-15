import { stringify } from 'smol-toml';
import { damageFixture } from './damage-fixture.js';
import { emptyFitting, fittingHardware, healthyEquipment } from '../workshop-fitting.js';
export function fittingFixture() {
  const result = damageFixture(), value = result.value, system = value.system.find(row => row.kind === 'phaser_bank');
  const hardware = fittingHardware(value, system), profile = emptyFitting();
  profile.slot.push({ id: 'beam-slot', system: system.id, family: 'beam', interface: 'beam', size: 1, default_equipment: 'standard', operation_component: system.damage_component.find(row => row.binding === 'operation').id });
  profile.equipment.push({ id: 'standard', interface: 'beam', size: 1, storage_space: 1, work: { class: 'field', work_secs: 10, manual_work_fraction: 0.25, requirements: [{ resource: 'machinery_parts', service: 'machinery_parts', amount: 1 }] }, hardware });
  profile.equipment.push({ ...structuredClone(profile.equipment[0]), id: 'efficient', hardware: { ...structuredClone(hardware), continuous_cost_per_sec: 0.5 } });
  profile.locker.push({ id: 'spares', compartment: value.interior.placements[system.id], space: 1 });
  profile.assembly.push({ id: 'installed', definition: 'standard', location: { kind: 'installed', slot: 'beam-slot' } },
    { id: 'spare', definition: 'efficient', location: { kind: 'stored', locker: 'spares' }, condition: healthyEquipment() });
  value.fitting = profile; return { ...result, value, profile, source: stringify(value) };
}
