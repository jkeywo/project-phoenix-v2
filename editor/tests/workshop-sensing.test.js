import fs from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { damageFixture } from './damage-fixture.js';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { resolveTemplate } from '../entity-includes.js';
import { inspectSensing, validateSensing, prepareSensingOperation, applySensingOperation, guardSensingReferences } from '../workshop-sensing.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
function fixture() { const out = damageFixture(); out.schema.console_instance_publication_kinds.passive_receiver = 'Passive'; return out; }
describe('accepted-source independent sensing instruments', () => {
  it('edits full profile through real acceptance with exact undo/export and no-write refusal', async () => {
    const { value, schema } = fixture(), before = '# header\n' + stringify(value) + '\n# tail\n', draft = make(before), profile = structuredClone(value.sensing); profile.focused.work_secs = 4;
    const runtime = { shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true })) };
    await applySensingOperation({ draft, runtime, dependencies: deps, operation: { path, value: profile } });
    const after = draft.read(path); expect(after).toContain('# tail\n'); expect(parse(after).energy).toEqual(value.energy);
    expect(inspectSensing(new WorkshopDocument(draft.archive()), deps, path).sensing).toEqual(profile);
    draft.undo(); expect(draft.read(path)).toBe(before); draft.redo();
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'instrument refused' }] }); profile.passive.mount.range = 500;
    await expect(applySensingOperation({ draft, runtime, dependencies: deps, operation: { path, value: profile } })).rejects.toThrow();
    expect(draft.read(path)).toBe(after); expect(draft.snapshot().history.undo).toHaveLength(1);
  });
  it('requires explicit materialisation and replaces the whole inherited profile', () => {
    const { value, schema } = fixture(), original = stringify(value), source = 'includes=["base.toml"]\n# child\n', draft = make(source), dependencies = { base_files: { [base]: original }, packs: [] };
    const profile = { passive: value.sensing.passive };
    expect(() => prepareSensingOperation(draft, dependencies, { path, value: profile }, schema)).toThrow(/materialise/);
    const after = prepareSensingOperation(draft, dependencies, { path, value: profile, materialise: true }, schema).changes[0].after;
    expect(resolveTemplate(path, { [path]: after, [base]: original }, parse).resolved.value.sensing).toEqual(profile); expect(draft.read(path)).toBe(source);
  });
  it('fits actual receiver hardware with finite reserve and physical room plus exact feed and lower-rating AI', () => {
    const { value, schema } = fixture(), profile = structuredClone(value.sensing), id = profile.passive.system;
    delete value.sensing.passive; value.system = value.system.filter(row => row.id !== id); delete value.interior.placements[id];
    value.energy.consumer = value.energy.consumer.filter(row => row.system !== id);
    for (const console of value.console) console.feed = console.feed?.filter(row => row.source !== id);
    for (const station of value.station) for (const rating of station.rating || []) for (const key of ['automated_systems', 'detailed_systems']) if (rating[key]) rating[key] = rating[key].filter(other => other !== id);
    const after = prepareSensingOperation(make(stringify(value)), deps, { path, type: 'sensing-fit-passive', value: profile, console: 'science', group: 'energy-passive-receiver', compartment: 'bridge' }, schema).changes[0].after, result = parse(after);
    expect(result.system.find(row => row.id === id).damage_component.map(row => row.binding)).toEqual(['operation', 'intake', 'cooling']);
    expect(result.energy.consumer.find(row => row.system === id).reserve.capacity).toBeGreaterThan(0); expect(result.interior.placements[id]).toBe('bridge');
    expect(result.console.find(row => row.id === 'science').feed).toContainEqual({ source: id, kind: 'Passive' });
    expect(result.station.find(row => row.id === 'science').rating.slice(1).every(row => row.automated_systems.includes(id))).toBe(true);
    validateSensing(result.sensing, result);
  });
  it.each([p => { p.focused.work_secs = 0; }, p => { p.focused.work_secs = '2'; }, p => { p.passive.mount.facing_deg = '90'; }, p => { p.broad.sample_interval_secs = 1e-46; }, p => { p.broad.assessment_secs = 0; }, p => { p.passive.mount.arc_deg = 361; }, p => { p.passive.bearing_error_rad = 4; }, p => { p.passive.system = 'repair'; }, p => { p.extension = true; }])('refuses invalid closed finite profiles', edit => {
    const { value } = fixture(), profile = structuredClone(value.sensing); edit(profile); expect(() => validateSensing(profile, value)).toThrow();
  });
  it('guards physical owner references and does not permit an orphan fitted receiver', () => {
    const { value } = fixture(), proposed = structuredClone(value); delete proposed.interior.placements[value.sensing.passive.system];
    expect(() => guardSensingReferences(value, proposed)).toThrow(/Sensing profile/);
    expect(() => validateSensing({ focused: value.sensing.focused }, value)).toThrow(/Every fitted/);
  });
  it('resolves curated independent hardware for both actual cruiser Console layouts', () => {
    for (const hull of ['alliance_cruiser', 'dynasty_player_cruiser']) {
      const result = resolveTemplate(`assets/entities/${hull}.toml`, { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse).resolved.value;
      validateSensing(result.sensing, result); const id = result.sensing.passive.system, system = result.system.find(row => row.id === id);
      expect(result.energy.consumer.filter(row => row.system === id)).toHaveLength(1);
      expect(result.console.find(row => row.id === system.console).feed).toContainEqual({ source: id, kind: 'Passive' });
      expect(result.sensing.focused.work_secs).toBe(2); expect(result.sensing.broad.system).not.toBe(id);
    }
  });
});
