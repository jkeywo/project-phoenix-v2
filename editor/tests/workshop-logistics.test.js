import { describe, it, expect, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { damageFixture } from './damage-fixture.js';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectLogistics, prepareLogisticsOperation, applyLogisticsOperation, validateLogistics } from '../workshop-logistics.js';
import { resolveTemplate } from '../entity-includes.js';
import { logisticsFixture } from './logistics-fixture.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
describe('accepted located logistics authoring', () => {
  it.each(['\n', '\r\n'])('edits a complete profile with source siblings and line endings preserved %j', nl => {
    const { source, value } = logisticsFixture(), before = source.replaceAll('\n', nl), draft = make(before);
    value.logistics.store[0].reserve = 2;
    const after = prepareLogisticsOperation(draft, deps, { path, logistics: value.logistics }).changes[0].after;
    expect(parse(after).logistics.store[0].reserve).toBe(2); expect(after.startsWith('# keep this header' + nl)).toBe(true);
    expect(after).toContain('# keep this tail' + nl + '[appearance]' + nl + 'color=[1,2,3]');
    expect(parse(after).torpedoes).toEqual(value.torpedoes); expect(draft.read(path)).toBe(before);
  });
  it('explicitly materialises an atomic inherited profile and removes old entries/provenance', () => {
    const { source, value } = logisticsFixture(), draft = make('includes=["base.toml"]\n# child\n'), dependencies = { base_files: { [base]: source }, packs: [] };
    const profile = structuredClone(value.logistics); profile.store = [profile.store[0]]; profile.storage = [profile.storage[0]];
    expect(() => prepareLogisticsOperation(draft, dependencies, { path, logistics: profile })).toThrow(/materialise/);
    const after = prepareLogisticsOperation(draft, dependencies, { path, logistics: profile, materialise: true }).changes[0].after;
    const resolved = resolveTemplate(path, { [path]: after, [base]: source }, parse);
    expect(resolved.resolved.value.logistics).toEqual(profile); expect(dependencies.base_files[base]).toBe(source);
    expect([...resolved.resolved.provenance.fields.keys()].some(key => key.includes('local'))).toBe(false);
  });
  it('validates before applying, roundtrips history/export and preserves refused source', async () => {
    const { source, value } = logisticsFixture(), draft = make(source), runtime = { validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    value.logistics.handling[0].max_batch = 3;
    await applyLogisticsOperation({ draft, runtime, dependencies: deps, operation: { path, logistics: value.logistics } });
    const after = draft.read(path); draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(draft.read(path)).toBe(after);
    expect(inspectLogistics(WorkshopDocument.restore(draft.snapshot()), deps, path).logistics.handling[0].max_batch).toBe(3);
    expect(inspectLogistics(new WorkshopDocument(draft.archive()), deps, path).logistics.handling[0].max_batch).toBe(3);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'actual route refused' }] });
    value.logistics.store[0].reserve = 1;
    await expect(applyLogisticsOperation({ draft, runtime, dependencies: deps, operation: { path, logistics: value.logistics } })).rejects.toThrow();
    expect(draft.read(path)).toBe(after); expect(draft.snapshot().history.undo).toHaveLength(1);
  });
  it.each([
    p => { p.store[0].target = 7; }, p => { p.store[1].source = 'missing'; }, p => { p.store[1].system = 'missing'; },
    p => { p.store[1].resource = 'fuel'; }, p => { p.store[0].system = 'inventory'; }, p => { p.store[0].endpoint.id = 'missing'; },
    p => { p.store.push({ ...p.store[0], id: 'duplicate' }); }, p => { p.store[0].compartment = 'missing'; },
    p => { p.storage[0].space = 3; }, p => { p.storage[0].unit_space.torpedoes = 0; },
    p => { p.handling[0].system = 'magazine'; }, p => { p.handling[0].units_per_sec = Infinity; },
    p => { p.handling[0].max_active = 65536; }, p => { p.handling.push(p.handling[0]); },
    p => { p.store[0].teleport = true; }, p => { p.handling[0].max_batch = 1.5; },
    p => { p.handling[0].max_batch = 1000000000; p.handling[0].units_per_sec = 1e-45; },
    p => { p.store[0].priority = 65536; }, p => { p.store[1].priority = -1; },
    p => { p.handling[0].reorder_fraction = -0.01; }, p => { p.handling[0].reorder_fraction = 1.01; },
  ])('refuses unsupported, dangling or impossible authored candidate %#', mutate => {
    const { value } = logisticsFixture(); mutate(value.logistics);
    expect(() => validateLogistics(value.logistics, value)).toThrow();
  });
  it('rejects stale validation without applying captured source', async () => {
    const { source, value } = logisticsFixture(), draft = make(source); let finish;
    const runtime = { validate: () => new Promise(resolve => { finish = resolve; }) };
    value.logistics.handling[0].max_batch = 3;
    const pending = applyLogisticsOperation({ draft, runtime, dependencies: deps, operation: { path, logistics: value.logistics } });
    await Promise.resolve(); draft.apply([{ path, before: source, after: source + '\n# concurrent\n' }]); finish({ accepted: true });
    await expect(pending).rejects.toThrow(/stale/); expect(draft.read(path)).toContain('# concurrent');
  });
  it('fits actual handling, reserve, faults, compartment, finite hold and explicit feeds atomically', () => {
    const { value, schema } = damageFixture(); schema.console_instance_publication_kinds.logistics = 'Logistics'; delete value.logistics;
    value.system = value.system.filter(row => row.id !== 'logistics'); delete value.interior.placements.logistics;
    for (const station of value.station) for (const rating of station.rating || []) for (const key of ['automated_systems', 'detailed_systems'])
      if (rating[key]) rating[key] = rating[key].filter(id => id !== 'logistics');
    delete value.station.find(row => row.id === 'engineering').rating[1].detailed_systems;
    value.energy.consumer = value.energy.consumer.filter(row => row.system !== 'logistics');
    const draft = make(stringify(value)), before = draft.read(path);
    const after = prepareLogisticsOperation(draft, deps, { path, type: 'logistics-create', system: 'new-handling',
      console: 'engineering', group: 'energy-logistics', compartment: 'engineering', capacity: 'new-hold', amount: 0, ceiling: 6 }, schema).changes[0].after;
    const result = parse(after), added = result.system.find(row => row.id === 'new-handling');
    expect(added.config.ai.tree[0].state[0].task[0].action.value).toEqual({ source: 'fact', name: 'delivery_available' });
    expect(added.damage_component.map(row => row.binding)).toEqual(['operation', 'intake', 'cooling']);
    expect(result.energy.consumer.some(row => row.system === 'new-handling')).toBe(true);
    expect(result.interior.placements['new-handling']).toBe('engineering');
    expect(result.infrastructure.capacity.find(row => row.id === 'new-hold')).toMatchObject({ amount: 0, ceiling: 6 });
    expect(result.logistics.store[1]).toMatchObject({ endpoint: { kind: 'torpedo_magazine' }, system: 'torpedo-magazine', compartment: 'weapons' });
    expect(result.console.find(row => row.id === 'engineering').feed).toContainEqual({ source: 'new-handling', kind: 'Logistics' });
    const ratings = result.station.find(row => row.id === 'engineering').rating;
    expect(ratings[0].detailed_systems).toContain('new-handling');
    expect(ratings.slice(1).every(row => row.automated_systems.includes('new-handling'))).toBe(true);
    expect(draft.read(path)).toBe(before);
  });
});
