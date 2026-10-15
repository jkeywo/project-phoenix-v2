import { expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { validateCommsRadio, inspectCommsRadio, prepareCommsRadioOperation, applyCommsRadioOperation, guardCommsRadioReferences } from '../workshop-comms-radio.js';
const path = 'assets/entities/test.toml', deps = { base_files: {}, packs: [] };
const entity = () => ({ name: 'Endpoint', comms: { range: 100, hailable: true, display_name: 'Public endpoint' },
  console: [{ id: 'radio-room' }], system: [{ id: 'comms', kind: 'comms', console: 'radio-room' }],
  energy: { consumer: [{ system: 'comms', continuous_cost_per_sec: .05, discrete_cost: 1 }] } });
const profile = () => ({ system: 'comms', request_reading_ticks: 45, keys: ['crew-key'], channel: [
  { id: 'open', range: 100, work_units: 1 }, { id: 'operations', range: 50, work_units: 2, key: 'crew-key' },
] });
const make = source => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="radio"\nname="Radio"\nversion="1"\n' }, { path, text: source }]));
it('validates exact equipment, finite endpoint-bounded channels, paid work and actually held keys', () => {
  validateCommsRadio(profile(), entity());
  for (const mutate of [p => p.system = 'other', p => p.extra = true, p => p.request_reading_ticks = 0, p => p.request_reading_ticks = 1.5, p => p.request_reading_ticks = 4294967296, p => p.keys = null, p => p.keys.push('crew-key'),
    p => p.keys = ['a'.repeat(129)], p => p.keys = ['é'.repeat(65)], p => p.keys = ['bad\nkey'],
    p => p.channel = [], p => p.channel.push(p.channel[0]), p => p.channel[0].id = 'BAD',
    p => p.channel[0].range = 101, p => p.channel[0].range = 1e-46, p => p.channel[0].range = Infinity,
    p => p.channel[0].work_units = 0, p => p.channel[0].work_units = 1.5, p => p.channel[0].work_units = 4294967296,
    p => p.channel[0].relay = 'yes', p => p.channel[1].key = 'not-held']) {
    const value = profile(); mutate(value); expect(() => validateCommsRadio(value, entity())).toThrow();
  }
  for (const mutate of [e => e.system[0].console = 'missing', e => e.system[0].kind = 'sensors', e => e.energy.consumer = [],
    e => e.energy.consumer[0].continuous_cost_per_sec = 0, e => e.energy.consumer[0].discrete_cost = 0, e => e.comms.range = NaN]) {
    const value = entity(); mutate(value); expect(() => validateCommsRadio(profile(), value)).toThrow();
  }
});
it('adds absent radio without changing endpoint, hailability, display name or commentary', async () => {
  const source = '# endpoint commentary\n' + stringify(entity()), draft = make(source), runtime = { validate: vi.fn(async () => ({ accepted: true })) };
  await applyCommsRadioOperation({ draft, runtime, dependencies: deps, operation: { path, value: profile() } });
  expect(draft.read(path).startsWith(source)).toBe(true);
  expect(parse(draft.read(path)).comms).toEqual({ ...entity().comms, radio: profile() });
  draft.undo(); expect(draft.read(path)).toBe(source); draft.redo();
  expect(inspectCommsRadio(new WorkshopDocument(draft.archive()), deps, path).radio).toEqual(profile());
});
it('materialises inherited channels and clears inherited protection/keys explicitly through history', async () => {
  const source = 'includes=["base.toml"]\n# child\n', parent = stringify({ ...entity(), comms: { ...entity().comms, radio: profile() } });
  const dependencies = { base_files: { 'assets/entities/base.toml': parent }, packs: [] }, draft = make(source);
  const value = { system: 'comms', request_reading_ticks: 45, keys: [], channel: [{ id: 'operations', range: 40, work_units: 3 }] };
  expect(() => prepareCommsRadioOperation(draft, dependencies, { path, value })).toThrow(/materialise/);
  await applyCommsRadioOperation({ draft, dependencies, runtime: { validate: async () => ({ accepted: true }) }, operation: { path, value, materialise: true } });
  expect(inspectCommsRadio(new WorkshopDocument(draft.archive()), dependencies, path).radio).toEqual(value);
  expect(inspectCommsRadio(draft, dependencies, path).resolved.value.comms.range).toBe(100);
  draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(draft.read(path)).toContain('# child');
});
it('preserves local radio and surrounding Comms comments and refuses header-like string corruption', () => {
  const source = stringify(entity()) + '\n[comms.radio] # channel policy\nsystem="comms"\nkeys=[]\n[[comms.radio.channel]] # clear channel\nid="open" # name\nrange=80\nwork_units=1\n';
  const after = prepareCommsRadioOperation(make(source), deps, { path, value: profile() }).changes[0].after;
  for (const comment of ['# channel policy', '# clear channel', '# name']) expect(after).toContain(comment);
  expect(parse(after).comms).toEqual({ ...entity().comms, radio: profile() });
  const tricky = 'description="""\n[comms.radio]\nsecret=1\n"""\n' + stringify(entity()), draft = make(tricky);
  expect(() => prepareCommsRadioOperation(draft, deps, { path, value: profile() })).toThrow(); expect(draft.read(path)).toBe(tricky);
});
it('refuses invalid acceptance and stale candidates before source/history mutation', async () => {
  const source = stringify(entity()), draft = make(source), runtime = { validate: vi.fn(async () => ({ accepted: false })) };
  const options = { draft, runtime, dependencies: deps, operation: { path, value: profile() } };
  await expect(applyCommsRadioOperation(options)).rejects.toThrow(); expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0);
  let answer; runtime.validate.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; }));
  const pending = applyCommsRadioOperation(options); await vi.waitFor(() => expect(answer).toBeTypeOf('function'));
  draft.apply([{ path, before: source, after: source + '# another edit\n' }]); answer({ accepted: true });
  await expect(pending).rejects.toThrow(/stale/); expect(draft.read(path)).toBe(source + '# another edit\n');
});
it('guards removal of the actual owner or its paid consumer', () => {
  const before = entity(); before.comms.radio = profile(); const after = structuredClone(before); after.system = [];
  expect(() => guardCommsRadioReferences(before, after)).toThrow(/installed Comms/);
  after.system = before.system; after.energy.consumer[0].discrete_cost = 0;
  expect(() => guardCommsRadioReferences(before, after)).toThrow(/positive/);
});

it('accepts explicit relay channel capability through the same history transaction', async () => {
  const source = stringify(entity()), draft = make(source), value = profile();
  value.channel[0].relay = true;
  await applyCommsRadioOperation({ draft, runtime: { validate: async () => ({ accepted: true }) }, dependencies: deps, operation: { path, value } });
  expect(inspectCommsRadio(new WorkshopDocument(draft.archive()), deps, path).radio.channel[0].relay).toBe(true);
  draft.undo(); expect(draft.read(path)).toBe(source); draft.redo();
  expect(parse(draft.read(path)).comms.radio.channel[0].relay).toBe(true);
});
