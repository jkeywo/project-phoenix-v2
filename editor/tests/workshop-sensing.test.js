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
  it('keeps curated sharing off by choice and refuses unsupported owner or malformed policy', () => {
    for(const hull of ['alliance_cruiser','dynasty_player_cruiser']) {
      const value=resolveTemplate(`assets/entities/${hull}.toml`,{read:file=>fs.existsSync(file)?fs.readFileSync(file,'utf8'):null},parse).resolved.value;
      validateSensing(value.sensing,value);
      expect(value.sensing.sharing).toEqual([{system:'sensor-radar',share_own_faction:false,receive:true},{system:'tactical-radar',share_own_faction:false,receive:true}]);
      for(const update of [{system:'helm-radar',share_own_faction:true,receive:true},{system:'sensor-radar',share_own_faction:'yes',receive:true}]) {
        expect(()=>validateSensing({...value.sensing,sharing:[update]},value)).toThrow(/Sharing/);
      }
      const profile=structuredClone(value.sensing);profile.focused[0].forecast={horizon_secs:Infinity};
      expect(()=>validateSensing(profile,value)).toThrow(/forecast/);
    }
  });
  it('requires explicit real actuator ownership and closed controller trees', () => {
    const {value}=fixture(), profile=structuredClone(value.sensing);
    delete profile.control.science_target;expect(()=>validateSensing(profile,value)).toThrow(/science_target/);
    profile.control.science_target='repair';expect(()=>validateSensing(profile,value)).toThrow(/owned sensors/);
    profile.control.science_target='sensors';const changed=structuredClone(value);delete changed.system.find(row=>row.id==='sensors').config.ai;
    expect(()=>validateSensing(profile,changed)).toThrow(/explicit closed/);
    profile.focused=profile.focused[0];expect(()=>validateSensing(profile,value)).toThrow(/arrays/);
  });
  it('authors independent radar arrays and refuses duplicate or incompatible fitted owners', () => {
    const { value, schema } = fixture(), before = stringify(value), profile = structuredClone(value.sensing);
    expect(profile.broad.map(row => row.system).sort()).toEqual(['helm-radar', 'sensor-radar', 'tactical-radar']);
    profile.broad.find(row => row.system === 'helm-radar').sample_interval_secs = 2;
    const after = prepareSensingOperation(make(before), deps, { path, value: profile }, schema).changes[0].after;
    expect(parse(after).sensing.broad).toEqual(profile.broad);
    expect(parse(after).energy).toEqual(value.energy);
    const duplicate = structuredClone(profile); duplicate.broad.push(duplicate.broad[0]); expect(() => validateSensing(duplicate, value)).toThrow(/distinct/);
    const wrong = structuredClone(profile); wrong.broad[0].system = 'repair'; expect(() => validateSensing(wrong, value)).toThrow(/kind/);
    const missing = structuredClone(value); delete missing.helm_console.radar; expect(() => validateSensing(profile, missing)).toThrow(/coverage/);
    expect(() => validateSensing({ ...profile, broad: profile.broad[0] }, value)).toThrow(/array/);
  });
  it('migrates existing NPC Tactical radars from unbounded to explicit finite authored coverage', () => {
    for (const [hull, range] of [['ship_harrow_cruiser',350],['ship_harrow_destroyer',400],['ship_harrow_patrol',110],['ship_harrow_tug',800],['ship_harrow_warhawk',320],['ship_requiem_courier',80]]) {
      const value = resolveTemplate(`assets/entities/${hull}.toml`, { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse).resolved.value;
      validateSensing(value.sensing, value); expect(value.sensing.broad.map(row => row.system)).toEqual(['tactical-radar']);
      expect(value.weapons_console.radar.range).toBe(range);
      expect(value.system.filter(row => row.kind === 'sensor_radar')).toHaveLength(0);
    }
  });
  it('edits full profile through real acceptance with exact undo/export and no-write refusal', async () => {
    const { value, schema } = fixture(), before = '# header\n' + stringify(value) + '\n# tail\n', draft = make(before), profile = structuredClone(value.sensing); profile.focused[0].work_secs = 4;
    const runtime = { shipSchema: async () => schema, validate: vi.fn(async () => ({ accepted: true })) };
    await applySensingOperation({ draft, runtime, dependencies: deps, operation: { path, value: profile } });
    const after = draft.read(path); expect(after).toContain('# tail\n'); expect(parse(after).energy).toEqual(value.energy);
    expect(inspectSensing(new WorkshopDocument(draft.archive()), deps, path).sensing).toEqual(profile);
    draft.undo(); expect(draft.read(path)).toBe(before); draft.redo();
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'instrument refused' }] }); profile.passive[0].mount.range = 500;
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
    const { value, schema } = fixture(), profile = structuredClone(value.sensing), id = profile.passive[0].system;
    delete value.sensing.passive; value.system = value.system.filter(row => row.id !== id); delete value.interior.placements[id];
    value.energy.consumer = value.energy.consumer.filter(row => row.system !== id);
    for (const console of value.console) console.feed = console.feed?.filter(row => row.source !== id);
    for (const station of value.station) for (const rating of station.rating || []) for (const key of ['automated_systems', 'detailed_systems']) if (rating[key]) rating[key] = rating[key].filter(other => other !== id);
    const after = prepareSensingOperation(make(stringify(value)), deps, { path, type: 'sensing-fit-passive', new_system: id, value: profile, console: 'science', group: 'energy-passive-receiver', compartment: 'bridge' }, schema).changes[0].after, result = parse(after);
    expect(result.system.find(row => row.id === id).damage_component.map(row => row.binding)).toEqual(['operation', 'intake', 'cooling']);
    expect(result.energy.consumer.find(row => row.system === id).reserve.capacity).toBeGreaterThan(0); expect(result.interior.placements[id]).toBe('bridge');
    expect(result.console.find(row => row.id === 'science').feed).toContainEqual({ source: id, kind: 'Passive' });
    expect(result.station.find(row => row.id === 'science').rating.slice(1).every(row => row.automated_systems.includes(id))).toBe(true);
    validateSensing(result.sensing, result);
  });
  it.each([p => { p.focused[0].work_secs = 0; }, p => { p.focused[0].work_secs = '2'; }, p => { p.passive[0].mount.facing_deg = '90'; }, p => { p.broad[0].sample_interval_secs = 1e-46; }, p => { p.broad[0].assessment_secs = 0; }, p => { p.passive[0].mount.arc_deg = 361; }, p => { p.passive[0].bearing_error_rad = 4; }, p => { p.passive[0].system = 'repair'; }, p => { p.extension = true; }])('refuses invalid closed finite profiles', edit => {
    const { value } = fixture(), profile = structuredClone(value.sensing); edit(profile); expect(() => validateSensing(profile, value)).toThrow();
  });
  it('guards physical owner references and does not permit an orphan fitted receiver', () => {
    const { value } = fixture(), proposed = structuredClone(value); delete proposed.interior.placements[value.sensing.passive[0].system];
    expect(() => guardSensingReferences(value, proposed)).toThrow(/Sensing profile/);
    expect(() => validateSensing({ focused: value.sensing.focused }, value)).toThrow(/Every fitted/);
  });
  it('resolves curated independent hardware for both actual cruiser Console layouts', () => {
    for (const hull of ['alliance_cruiser', 'dynasty_player_cruiser']) {
      const result = resolveTemplate(`assets/entities/${hull}.toml`, { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse).resolved.value;
      validateSensing(result.sensing, result); const id = result.sensing.passive[0].system, system = result.system.find(row => row.id === id);
      expect(result.energy.consumer.filter(row => row.system === id)).toHaveLength(1);
      expect(result.console.find(row => row.id === system.console).feed).toContainEqual({ source: id, kind: 'Passive' });
      expect(result.sensing.focused[0].work_secs).toBe(2); expect(result.sensing.broad[0].system).not.toBe(id);
    }
  });
  it('migrates every remaining fitted broad radar explicitly while preserving actual hull reach', () => {
    for (const [hull, range] of [['alliance_battleship', 300], ['alliance_courier', 250], ['alliance_destroyer', 250]]) {
      const result = resolveTemplate(`assets/entities/${hull}.toml`, { read: file => fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null }, parse).resolved.value;
      validateSensing(result.sensing, result); expect(result.sensing.broad[0].system).toBe('sensor-radar');
      expect(result.sensing.broad[0].assessment_secs).toBeGreaterThan(result.sensing.broad[0].identity_secs);
      expect(result.sensors_console.long_range_radar.range).toBe(range);
      const invalid = structuredClone(result); delete invalid.sensors_console;
      expect(() => validateSensing(result.sensing, invalid)).toThrow(/radar coverage/);
    }
  });
});
