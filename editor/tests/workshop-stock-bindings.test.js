import fs from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { damageFixture } from './damage-fixture.js';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { applyLogisticsOperation, prepareLogisticsOperation } from '../workshop-logistics.js';
import { stockPublicationSources, requiredStockFeeds, guardLogisticsLayout, guardLogisticsSystemRemoval, guardManagedStockBounds } from '../workshop-stock-bindings.js';
import { resolveTemplate } from '../entity-includes.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml';
const make = source => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
function fixture() {
  const result = damageFixture(); result.schema.console_stock_publication_kind = 'Stock';
  if (!result.schema.console_feed_kinds.includes('Stock')) result.schema.console_feed_kinds.push('Stock');
  return result;
}
describe('closed stock bindings and authoring continuity', () => {
  it('expands typed sources from actual local owners only and deduplicates co-owned feeds', () => {
    const { value, schema } = fixture();
    expect(stockPublicationSources(value, schema)).toEqual(['stock/power-reactor', 'stock/repair', 'stock/torpedo-magazine']);
    expect(stockPublicationSources(value, {})).toEqual([]);
    const feeds = requiredStockFeeds(value, schema);
    expect(feeds.filter(row => row.source === 'stock/repair')).toEqual([{ console: 'engineering', source: 'stock/repair', kind: 'Stock' }]);
    expect(feeds.filter(row => row.source === 'stock/torpedo-magazine')).toHaveLength(2);
    value.logistics.store.push({ role: 'local', system: 'unknown' });
    expect(stockPublicationSources(value, schema)).not.toContain('stock/unknown');
  });
  it('adds required own/Inventory feeds in the same accepted-source transaction and preserves refusal/history', async () => {
    const { value, schema } = fixture();
    for (const row of value.console) row.feed = row.feed?.filter(feed => feed.kind !== 'Stock');
    const before = stringify(value), draft = make(before), runtime = { shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true })) };
    await applyLogisticsOperation({ draft, runtime, dependencies: { base_files: {}, packs: [] }, operation: { path, logistics: value.logistics } });
    const after = draft.read(path), parsed = parse(after);
    for (const feed of requiredStockFeeds(parsed, schema)) expect(parsed.console.find(row => row.id === feed.console).feed).toContainEqual({ source: feed.source, kind: feed.kind });
    expect(runtime.validate).toHaveBeenCalledOnce(); expect(draft.snapshot().history.undo).toHaveLength(1);
    draft.undo(); expect(draft.read(path)).toBe(before); draft.redo(); expect(draft.read(path)).toBe(after);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'stock ownership refused' }] });
    const profile = structuredClone(value.logistics); profile.store[0].priority = 3;
    await expect(applyLogisticsOperation({ draft, runtime, dependencies: { base_files: {}, packs: [] }, operation: { path, logistics: profile } })).rejects.toThrow();
    expect(draft.read(path)).toBe(after); expect(draft.snapshot().history.undo).toHaveLength(1);
  });
  it('requires explicit inherited Console materialisation without changing its source or existing feeds', () => {
    const { value, schema } = fixture(); for (const row of value.console) row.feed = row.feed?.filter(feed => feed.kind !== 'Stock');
    const baseSource = stringify(value), source = 'includes=["base.toml"]\n# child\n', draft = make(source), dependencies = { base_files: { [base]: baseSource }, packs: [] };
    expect(() => prepareLogisticsOperation(draft, dependencies, { path, logistics: value.logistics }, schema)).toThrow(/materialise/);
    const after = prepareLogisticsOperation(draft, dependencies, { path, logistics: value.logistics, materialise: true }, schema).changes[0].after;
    expect(after).toContain('# child'); expect(dependencies.base_files[base]).toBe(baseSource);
    const resolved = resolveTemplate(path, { [path]: after, [base]: baseSource }, parse).resolved.value;
    for (const console of value.console) for (const feed of console.feed || []) expect(resolved.console.find(row => row.id === console.id).feed).toContainEqual(feed);
    for (const feed of requiredStockFeeds(value, schema)) expect(resolved.console.find(row => row.id === feed.console).feed).toContainEqual({ source: feed.source, kind: feed.kind });
  });
  it('refuses dangling equipment/room references and stock changes that would exceed authored limits', () => {
    const { value } = fixture();
    expect(() => guardLogisticsSystemRemoval(value, 'repair')).toThrow(/Reassign/);
    expect(() => guardLogisticsSystemRemoval(value, 'logistics')).toThrow(/Reassign/);
    expect(() => guardLogisticsSystemRemoval(value, 'sensors')).not.toThrow();
    const layout = structuredClone(value.interior); layout.placements.repair = 'support';
    expect(() => guardLogisticsLayout(value, layout)).toThrow(/Reassign/);
    expect(() => guardManagedStockBounds(value, { kind: 'capacity', id: 'machinery_parts' }, 12, 11)).toThrow(/target/);
    expect(() => guardManagedStockBounds(value, { kind: 'capacity', id: 'machinery_parts' }, 13, 13)).toThrow(/storage/);
    expect(() => guardManagedStockBounds(value, { kind: 'capacity', id: 'machinery_parts' }, 12, 12)).not.toThrow();
  });
  it('prunes only disappeared Stock publications atomically when removing the last store for an owner', async () => {
    const { value, schema } = fixture(), before = stringify(value), draft = make(before);
    const profile = structuredClone(value.logistics);
    profile.store = profile.store.filter(row => row.system !== 'repair');
    await applyLogisticsOperation({ draft, runtime: { shipSchema: async () => schema, validate: async () => ({ accepted: true }) },
      dependencies: { base_files: {}, packs: [] }, operation: { path, logistics: profile } });
    const changed = parse(draft.read(path));
    expect(changed.console.flatMap(row => row.feed || []).some(feed => feed.source === 'stock/repair')).toBe(false);
    expect(changed.console.flatMap(row => row.feed || []).some(feed => feed.source === 'stock/power-reactor')).toBe(true);
    expect(changed.console.flatMap(row => row.feed || []).some(feed => feed.source === 'repair' && feed.kind === 'Repair')).toBe(true);
    expect(changed.system).toEqual(value.system); expect(changed.infrastructure).toEqual(value.infrastructure);
    expect(draft.snapshot().history.undo).toHaveLength(1); draft.undo(); expect(draft.read(path)).toBe(before);
  });
  it('prunes inherited obsolete feeds only with explicit Console materialisation', () => {
    const { value, schema } = fixture(), baseSource = stringify(value), source = 'includes=["base.toml"]\n', draft = make(source);
    const dependencies = { base_files: { [base]: baseSource }, packs: [] }, profile = structuredClone(value.logistics);
    profile.store = profile.store.filter(row => row.system !== 'repair');
    const after = prepareLogisticsOperation(draft, dependencies, { path, logistics: profile, materialise: true }, schema).changes[0].after;
    const changed = resolveTemplate(path, { [path]: after, [base]: baseSource }, parse).resolved.value;
    expect(changed.console.flatMap(row => row.feed || []).some(feed => feed.source === 'stock/repair')).toBe(false);
    expect(changed.console.flatMap(row => row.feed || []).some(feed => feed.source === 'stock/torpedo-magazine')).toBe(true);
    expect(dependencies.base_files[base]).toBe(baseSource); expect(draft.read(path)).toBe(source);
  });
  it.each([['alliance_cruiser', 114, 102], ['dynasty_player_cruiser', 111, 99]])('conserves actual %s finite local+hold stocks, ceilings, roles and services', (hull, machinery, mainMachinery) => {
    const resolved = resolveTemplate(`assets/entities/${hull}.toml`, { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse);
    expect(resolved.ok).toBe(true); const value = resolved.resolved.value;
    const capacities = new Map(value.infrastructure.capacity.map(row => [row.id, row]));
    for (const [resource, total] of [['machinery_parts', machinery], ['structural_parts', 59], ['coolant', 780]]) {
      const stores = value.logistics.store.filter(row => row.resource === resource);
      expect(stores.reduce((sum, row) => sum + capacities.get(row.endpoint.id).amount, 0)).toBe(total);
      for (const row of stores) expect(capacities.get(row.endpoint.id).ceiling).toBe(capacities.get(row.endpoint.id).amount);
      expect(value.resource_services[resource]).toEqual({ kind: 'capacity', id: resource });
    }
    expect(capacities.get('hold-machinery-parts').amount).toBe(mainMachinery);
    expect(value.logistics.store.filter(row => row.role === 'main').every(row => row.compartment === 'support')).toBe(true);
    expect(value.logistics.handling[0]).toMatchObject({ max_active: 2, max_batch: 100, units_per_sec: 50, reorder_fraction: 0.5 });
    expect(value.logistics.store.find(row => row.id === 'magazine').priority).toBe(2);
    expect(value.torpedoes.initial_count + capacities.get('hold-ammunition').amount).toBe(6);
    expect(value.station).toHaveLength(6);
    for (const feed of requiredStockFeeds(value, { console_stock_publication_kind: 'Stock' })) expect(value.console.find(row => row.id === feed.console).feed).toContainEqual({ source: feed.source, kind: 'Stock' });
  });
});
