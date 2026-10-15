import fs from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { damageFixture } from './damage-fixture.js';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectProduction, validateProduction, prepareProductionOperation, applyProductionOperation } from '../workshop-production.js';
import { resolveTemplate } from '../entity-includes.js';
import { prepareLogisticsOperation } from '../workshop-logistics.js';
import { prepareShipOperation } from '../workshop-ship-authoring.js';
import { prepareResourceOperation } from '../workshop-resources.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
const fixture = () => { const result = damageFixture(); result.schema.console_stock_publication_kind = 'Stock';
  result.schema.console_instance_publication_kinds.fabrication = 'Production'; return result; };
describe('accepted-source production recipes', () => {
  it('edits closed recipes through one authoritative history step and retains exact unaffected source', async () => {
    const { value, schema } = fixture(), before = '# header\n' + stringify(value) + '\n# tail\n', draft = make(before);
    const profile = structuredClone(value.production); profile.recipe[0].work_secs = 12;
    const runtime = { shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true })) };
    await applyProductionOperation({ draft, runtime, dependencies: deps, operation: { path, production: profile } });
    const after = draft.read(path); expect(after.startsWith('# header\n')).toBe(true); expect(after).toContain('# tail\n');
    expect(parse(after).production.recipe[0].work_secs).toBe(12); expect(parse(after).infrastructure).toEqual(value.infrastructure);
    draft.undo(); expect(draft.read(path)).toBe(before); draft.redo(); expect(draft.read(path)).toBe(after);
    expect(inspectProduction(new WorkshopDocument(draft.archive()), deps, path).production).toEqual(profile);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'physical recipe refused' }] });
    profile.initial_target = 5;
    await expect(applyProductionOperation({ draft, runtime, dependencies: deps, operation: { path, production: profile } })).rejects.toThrow();
    expect(draft.read(path)).toBe(after); expect(draft.snapshot().history.undo).toHaveLength(1);
  });
  it('explicitly materialises one atomic profile while preserving immutable include definitions', () => {
    const { value, schema } = fixture(), baseSource = stringify(value), source = 'includes=["base.toml"]\n# child\n', draft = make(source), dependencies = { base_files: { [base]: baseSource }, packs: [] };
    const profile = structuredClone(value.production); profile.recipe[0].id = 'new-round'; profile.initial_recipe = 'new-round';
    expect(() => prepareProductionOperation(draft, dependencies, { path, production: profile }, schema)).toThrow(/materialise/);
    const after = prepareProductionOperation(draft, dependencies, { path, production: profile, materialise: true }, schema).changes[0].after;
    const resolved = resolveTemplate(path, { [path]: after, [base]: baseSource }, parse);
    expect(resolved.resolved.value.production).toEqual(profile);
    expect([...resolved.resolved.provenance.fields.keys()].some(key => key.includes('recipe[id=torpedoes]'))).toBe(false);
    expect(dependencies.base_files[base]).toBe(baseSource); expect(draft.read(path)).toBe(source);
  });
  it('fits actual powered machinery, conditions, physical placement, AI, ratings and permitted feeds atomically', () => {
    const { value, schema } = fixture(), profile = structuredClone(value.production); delete value.production;
    value.system = value.system.filter(row => row.id !== 'fabrication'); delete value.interior.placements.fabrication;
    value.energy.consumer = value.energy.consumer.filter(row => row.system !== 'fabrication');
    for (const console of value.console) console.feed = console.feed?.filter(row => row.source !== 'fabrication');
    for (const station of value.station) for (const rating of station.rating || []) for (const key of ['automated_systems', 'detailed_systems']) if (rating[key]) rating[key] = rating[key].filter(id => id !== 'fabrication');
    const draft = make(stringify(value)), after = prepareProductionOperation(draft, deps, { path, type: 'production-create', production: profile,
      console: 'engineering', group: 'energy-fabrication', compartment: 'support' }, schema).changes[0].after, result = parse(after);
    expect(result.system.find(row => row.id === 'fabrication').damage_component.map(row => row.binding)).toEqual(['operation', 'intake', 'cooling']);
    expect(result.system.find(row => row.id === 'fabrication').config.ai.tree[0].state[0].task[0].when[0].value.name).toBe('enabled');
    expect(result.energy.consumer.find(row => row.system === 'fabrication')).toBeTruthy(); expect(result.interior.placements.fabrication).toBe('support');
    expect(result.console.find(row => row.id === 'engineering').feed).toContainEqual({ source: 'fabrication', kind: 'Production' });
    expect(result.station.find(row => row.id === 'engineering').rating.slice(1).every(row => row.automated_systems.includes('fabrication'))).toBe(true);
    expect(result.infrastructure).toEqual(value.infrastructure);
  });
  it.each([
    p => { p.recipe[0].inputs = []; }, p => { p.recipe[0].outputs[0].store = 'repair-machinery'; },
    p => { p.recipe[0].inputs[0].amount = 0; }, p => { p.recipe[0].inputs.push(p.recipe[0].inputs[0]); },
    p => { p.recipe[0].outputs[0].amount = 7; }, p => { p.recipe[0].work_secs = 1e-46; },
    p => { p.initial_target = 7; }, p => { p.initial_recipe = null; p.initial_target = 1; },
    p => { p.recipe[0].code = 'run'; }, p => { p.system = 'repair'; },
  ])('rejects unsupported or impossible recipe %# before a draft change', mutate => {
    const { value } = fixture(), profile = structuredClone(value.production); mutate(profile);
    expect(() => validateProduction(profile, value)).toThrow();
  });
  it('guards recipe stores and fitted owner against dangling source edits', () => {
    const { value, schema } = fixture(), draft = make(stringify(value)), profile = structuredClone(value.logistics);
    profile.store = profile.store.filter(row => row.id !== 'hold-alloy');
    expect(() => prepareLogisticsOperation(draft, deps, { path, logistics: profile }, schema)).toThrow(/Production first/);
    expect(() => prepareShipOperation(draft, deps, schema, { path, type: 'system-update', id: 'fabrication', fields: {}, compartment: 'engineering' })).toThrow(/Production first/);
    value.logistics.store.find(row => row.id === 'hold-alloy').target = 0;
    expect(() => prepareResourceOperation(make(stringify(value)), deps, { path, type: 'capacity-set', id: 'alloy', amount: 0, ceiling: 1 })).toThrow(/Production first/);
    expect(draft.snapshot().history.undo).toHaveLength(0);
  });
  it.each(['alliance_cruiser', 'dynasty_player_cruiser'])('authors actual finite %s fabrication without adding seats or ammunition', hull => {
    const value = resolveTemplate(`assets/entities/${hull}.toml`, { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse).resolved.value;
    expect(() => validateProduction(value.production, value)).not.toThrow();
    expect(value.production).toMatchObject({ system: 'fabrication', initial_target: 0 });
    expect(value.production.recipe.find(row => row.id === 'torpedoes')).toMatchObject({ work_secs: 10, inputs: [{ store: 'hold-machinery-parts', amount: 1 }, { store: 'hold-alloy', amount: 2 }], outputs: [{ store: 'hold-ammunition', amount: 1 }] });
    expect(value.infrastructure.capacity.find(row => row.id === 'alloy')).toMatchObject({ amount: 12, ceiling: 24 });
    expect(value.resource_services.alloy).toEqual({ kind: 'capacity', id: 'alloy' }); expect(value.umbilical.allowed_resources).toContain('alloy');
    expect(value.torpedoes.initial_count + value.infrastructure.capacity.find(row => row.id === 'hold-ammunition').amount).toBe(6);
    expect(value.station).toHaveLength(6); expect(value.interior.placements.fabrication).toBe('support');
  });
});
