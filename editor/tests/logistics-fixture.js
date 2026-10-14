import { stringify } from 'smol-toml';
export function logisticsFixture() {
  const value = { name: 'Test', torpedoes: { count: 6, initial_count: 2 },
    infrastructure: { capacity: [{ id: 'hold', amount: 4, ceiling: 6 }] },
    system: [{ id: 'inventory', kind: 'logistics', console: 'engineering' }, { id: 'magazine', kind: 'torpedo_magazine', console: 'tactical' }],
    interior: { compartment: [{ id: 'hold' }, { id: 'weapons' }], placements: { inventory: 'hold', magazine: 'weapons' } },
    logistics: { store: [
      { id: 'hold', endpoint: { kind: 'capacity', id: 'hold' }, resource: 'torpedoes', compartment: 'hold', role: 'main', target: 4, reserve: 0 },
      { id: 'local', endpoint: { kind: 'torpedo_magazine' }, resource: 'torpedoes', compartment: 'weapons', role: 'local', system: 'magazine', source: 'hold', target: 6, reserve: 0 },
    ], storage: [{ compartment: 'hold', space: 6, unit_space: { torpedoes: 1 } }, { compartment: 'weapons', space: 6, unit_space: { torpedoes: 1 } }],
    handling: [{ system: 'inventory', max_active: 1, max_batch: 2, units_per_sec: 1 }] } };
  return { value, source: '# keep this header\n' + stringify(value) + '\n# keep this tail\n[appearance]\ncolor=[1,2,3]\n' };
}
