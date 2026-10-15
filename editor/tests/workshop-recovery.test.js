import fs from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { damageFixture } from './damage-fixture.js';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectRecovery, validateRecoveryTool, validateRecoverableSource, prepareRecoveryOperation, applyRecoveryOperation, guardRecoveryReferences } from '../workshop-resource-recovery.js';
import { resolveTemplate } from '../entity-includes.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
const fixture = () => { const out = damageFixture(); out.schema.console_stock_publication_kind = 'Stock';
  Object.assign(out.schema.console_instance_publication_kinds, { 'resource-recovery': 'Recovery', logistics: 'Logistics', fabrication: 'Production' }); return out; };
describe('accepted-source recovery declarations', () => {
  it('edits one atomic profile with exact history, persistence and authoritative refusal', async () => {
    const { value, schema } = fixture(), before = '# header\n' + stringify(value) + '\n# tail\n', draft = make(before), profile = { ...value.recovery, range: 9 };
    const runtime = { shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true })) };
    await applyRecoveryOperation({ draft, runtime, dependencies: deps, operation: { path, value: profile } });
    const after = draft.read(path); expect(after).toContain('# tail\n'); expect(parse(after).infrastructure).toEqual(value.infrastructure);
    expect(parse(after).recovery.range).toBe(9); draft.undo(); expect(draft.read(path)).toBe(before); draft.redo();
    expect(inspectRecovery(new WorkshopDocument(draft.archive()), deps, path).recovery).toEqual(profile);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'tool refused' }] });
    await expect(applyRecoveryOperation({ draft, runtime, dependencies: deps, operation: { path, value: { ...profile, range: 10 } } })).rejects.toThrow();
    expect(draft.read(path)).toBe(after); expect(draft.snapshot().history.undo).toHaveLength(1);
  });
  it('materialises inherited profiles explicitly without leaking replaced provenance', () => {
    const { value, schema } = fixture(), original = stringify(value), source = 'includes=["base.toml"]\n# child\n', draft = make(source), dependencies = { base_files: { [base]: original }, packs: [] };
    const profile = { ...value.recovery, resources: ['ore'] };
    expect(() => prepareRecoveryOperation(draft, dependencies, { path, value: profile }, schema)).toThrow(/materialise/);
    const after = prepareRecoveryOperation(draft, dependencies, { path, value: profile, materialise: true }, schema).changes[0].after;
    expect(resolveTemplate(path, { [path]: after, [base]: original }, parse).resolved.value.recovery).toEqual(profile); expect(draft.read(path)).toBe(source);
  });
  it('fits one real System with reserve, faults, placement, lower-rating AI and only explicit selected-contact knowledge', () => {
    const { value, schema } = fixture(), profile = structuredClone(value.recovery); delete value.recovery;
    value.system = value.system.filter(row => row.id !== profile.system); delete value.interior.placements[profile.system];
    value.energy.consumer = value.energy.consumer.filter(row => row.system !== profile.system);
    for (const console of value.console) console.feed = console.feed?.filter(row => row.source !== profile.system);
    for (const station of value.station) for (const rating of station.rating || []) for (const key of ['automated_systems', 'detailed_systems']) if (rating[key]) rating[key] = rating[key].filter(id => id !== profile.system);
    const draft = make(stringify(value)); const after = prepareRecoveryOperation(draft, deps, { path, type: 'recovery-create', value: profile,
      console: 'engineering', group: 'energy-resource-recovery', compartment: 'support' }, schema).changes[0].after, result = parse(after);
    expect(result.system.find(row => row.id === profile.system).damage_component.map(row => row.binding)).toEqual(['operation', 'intake', 'cooling']);
    expect(result.energy.consumer.find(row => row.system === profile.system)).toBeTruthy(); expect(result.interior.placements[profile.system]).toBe('support');
    const feeds = result.console.find(row => row.id === 'engineering').feed;
    expect(feeds).toContainEqual({ source: 'sensor-radar', kind: 'SensorRadar' }); expect(feeds).toContainEqual({ source: profile.system, kind: 'Recovery' });
    expect(feeds.filter(row => row.kind === 'TacticalRadar')).toEqual(value.console.find(row => row.id === 'engineering').feed.filter(row => row.kind === 'TacticalRadar'));
    expect(result.station.find(row => row.id === 'engineering').rating.slice(1).every(row => row.automated_systems.includes(profile.system))).toBe(true);
  });
  it.each([p => { p.range = 0; }, p => { p.units_per_sec = 1e-46; }, p => { p.resources.push(p.resources[0]); }, p => { p.system = 'repair'; }, p => { p.unknown = true; }])('rejects closed/unsupported tool definitions', edit => {
    const { value } = fixture(), profile = structuredClone(value.recovery); edit(profile); expect(() => validateRecoveryTool(profile, value)).toThrow();
  });
  it('preserves an explicitly authored alternative Tactical feed without silently adding Science knowledge', () => {
    const { value: original, schema } = fixture();
    const value = JSON.parse(JSON.stringify(original).replaceAll('tactical-radar', 'custom-tactical-array'));
    for (const row of value.console) for (const feed of row.feed || []) if (feed.kind === 'TacticalRadar') feed.source = 'tactical-radar';
    const console = value.console.find(row => row.id === 'engineering');
    console.feed = console.feed.filter(row => row.kind !== 'SensorRadar'); console.feed.push({ source: 'tactical-radar', kind: 'TacticalRadar' });
    const after = prepareRecoveryOperation(make(stringify(value)), deps, { path, value: value.recovery }, schema).changes[0].after;
    expect(parse(after).console.find(row => row.id === 'engineering').feed).toEqual(console.feed);
  });
  it('uses the fixed Science publication when fitting against a custom SensorRadar System ID', () => {
    const { value: original, schema } = fixture();
    const value = JSON.parse(JSON.stringify(original).replaceAll('sensor-radar', 'custom-science-array'));
    for (const row of value.console) for (const feed of row.feed || []) if (feed.kind === 'SensorRadar') feed.source = 'sensor-radar';
    const console = value.console.find(row => row.id === 'engineering'); console.feed = console.feed.filter(row => !['SensorRadar', 'TacticalRadar'].includes(row.kind));
    const after = prepareRecoveryOperation(make(stringify(value)), deps, { path, value: value.recovery }, schema).changes[0].after;
    expect(parse(after).console.find(row => row.id === 'engineering').feed).toContainEqual({ source: 'sensor-radar', kind: 'SensorRadar' });
    expect(parse(after).console.find(row => row.id === 'engineering').feed.some(row => row.source === 'custom-science-array')).toBe(false);
  });
  it('binds finite endpoints without authored quantity counters and prevents dangling source stock removal', () => {
    const { value } = fixture(); const source = { availability: 'always', resources: { scrap: { kind: 'capacity', id: 'scrap' } } };
    validateRecoverableSource(source, value); expect(() => validateRecoverableSource({ ...source, resources: { scrap: source.resources.scrap, ore: source.resources.scrap } }, value)).toThrow(/alias/);
    expect(() => validateRecoverableSource({ ...source, quantity: 10 }, value)).toThrow();
    value.recoverable = source; const proposed = structuredClone(value); proposed.infrastructure.capacity = proposed.infrastructure.capacity.filter(row => row.id !== 'scrap');
    expect(() => guardRecoveryReferences(value, proposed)).toThrow(/Recovery first/);
  });
  it('authors finite actual sources and keeps new receiving stocks empty, original ammo unchanged and processing optional', () => {
    for (const hull of ['alliance_cruiser', 'dynasty_player_cruiser']) {
      const name = `assets/entities/${hull}.toml`, result = resolveTemplate(name, { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse).resolved.value;
      expect(result.torpedoes.initial_count + result.infrastructure.capacity.find(row => row.id === 'hold-ammunition').amount).toBe(6);
      for (const resource of ['ore', 'scrap']) expect(result.infrastructure.capacity.find(row => row.id === resource)).toMatchObject({ amount: 0, ceiling: 24 });
      for (const source of ['recovery_cargo_pod', 'recovery_ore_deposit']) {
        const target = resolveTemplate(`assets/entities/${source}.toml`, { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse).resolved.value.target;
        expect(target.tags.some(tag => result.sensors_console.long_range_radar.shows.includes(tag))).toBe(true);
        expect(target.tags.some(tag => result.sensors_console.long_range_radar.selects.includes(tag))).toBe(true);
      }
      expect(result.production.initial_target).toBe(0); expect(result.production.recipe.map(row => row.id)).toEqual(['torpedoes', 'ore-to-alloy', 'scrap-to-alloy']);
      expect(result.logistics.storage.find(row => row.compartment === 'support').space).toBe(hull === 'alliance_cruiser' ? 855 : 852);
    }
    const pod = parse(fs.readFileSync('assets/entities/recovery_cargo_pod.toml', 'utf8'));
    expect(pod.infrastructure.capacity.map(row => [row.id, row.amount])).toEqual([['scrap', 12], ['machinery_parts', 4]]); validateRecoverableSource(pod.recoverable, pod);
    expect(parse(fs.readFileSync('assets/entities/recovery_ore_deposit.toml', 'utf8')).infrastructure.capacity[0].amount).toBe(24);
  });
});
