import { describe, it, expect, vi } from 'vitest';
import { parse } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { defaultWeapon, inspectWeapons, prepareWeaponOperation, applyWeaponOperation } from '../workshop-weapons.js';
import { damageFixture } from './damage-fixture.js';
import { resolveTemplate } from '../entity-includes.js';
const path = 'assets/entities/ship.toml', base = 'assets/entities/base.toml';
const dependencies = { base_files: {}, packs: [] };
const source = `# root note\n[[weapon]]\nid="beam-a"\nstrategy_id="a"\n[weapon.impact]\ndamage_type="laser"\npenetration=20\n[weapon.mount]\nfacing_deg=0\nfire_arc_deg=90\nrange=40\n[weapon.delivery]\nkind="beam"\nauto_arc_deg=90\ndamage_per_sec=10\nduration_secs=1\ncooldown_secs=1\n[weapon.ai]\nidle=true\n[[weapon]]\nid="bolt-b"\n[weapon.impact]\ndamage_type="laser"\npenetration=20\n[weapon.mount]\nfacing_deg=90\nfire_arc_deg=30\nrange=20\n[weapon.delivery]\nkind="bolt"\ndamage=3\n# next section note\n[appearance]\ncolor=[1,2,3]\n`;
function draft(text = source) { return new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text }])); }
function operation(current, fields, deps = dependencies) { return prepareWeaponOperation(current, deps, { path, ...fields }).changes[0].after; }
function edited(current, deps = dependencies) { const value = inspectWeapons(current, deps, path).weapons[0].weapon; value.mount.range = 50; return value; }
describe('Workshop configurable weapon source editing', () => {
  it.each(['\n', '\r\n'])('edits one complete weapon and preserves unrelated source %j', nl => {
    const before = source.replace(/\n/g, nl), current = draft(before);
    const after = operation(current, { section: 'weapon', originalId: 'beam-a', weapon: edited(current) });
    expect(after).toContain('# root note' + nl);
    expect(after.slice(after.indexOf('[[weapon]]', after.indexOf('[[weapon]]') + 1))).toBe(before.slice(before.indexOf('[[weapon]]', before.indexOf('[[weapon]]') + 1)));
    expect(parse(after).weapon[0]).toMatchObject({ strategy_id: 'a', mount: { range: 50 }, ai: { idle: true } });
  });
  it('creates supported generic weapons without a second legacy definition', () => {
    const value = defaultWeapon('bolt'); value.id = 'new'; value.barrels = ['left', 'right']; value.pattern = [{ offset_secs: 0, barrels: [0, 1] }];
    expect(() => operation(draft('# preserve\n'), { weapon: value })).toThrow(/Damage|Energy|Interiors/);
    const fixture = damageFixture();
    const after = operation(draft(fixture.source), { weapon: value, schema: fixture.schema, console: fixture.value.console[0].id, power_group: fixture.value.energy.group[0].id, compartment: fixture.value.interior.compartment[0].id });
    const result = parse(after); expect(result.weapon.at(-1)).toEqual(value);
    expect(result.system.find(system => system.id === 'blaster-new')).toMatchObject({ kind: 'blaster_bank', console: fixture.value.console[0].id });
    expect(result.energy.consumer.some(row => row.system === 'blaster-new')).toBe(true);
    expect(result.interior.placements['blaster-new']).toBe(fixture.value.interior.compartment[0].id);
  });
  it('requires materialisation and leaves includes and dependency bytes intact', () => {
    const current = draft('includes=["base.toml"]\n# child\n'), deps = { base_files: { [base]: source }, packs: [] }, weapon = edited(current, deps);
    expect(() => operation(current, { section: 'weapon', originalId: 'beam-a', weapon }, deps)).toThrow(/materialise/);
    const after = operation(current, { section: 'weapon', originalId: 'beam-a', weapon, materialise: true }, deps);
    expect(after.startsWith('includes=["base.toml"]\n# child\n')).toBe(true);
    const result = resolveTemplate(path, { [path]: after, [base]: source }, parse);
    expect(result.ok).toBe(true); expect(result.resolved.value.weapon).toHaveLength(2); expect(result.resolved.value.weapon[0].mount.range).toBe(50);
    expect(deps.base_files[base]).toBe(source);
  });
  it('replaces delivery variants atomically and clears obsolete provenance', () => {
    const result = resolveTemplate(path, { [base]: source, [path]: 'includes=["base.toml"]\n[[weapon]]\nid="beam-a"\n[weapon.delivery]\nkind="guided"\nvolley_max=2\n' }, parse);
    expect(result.resolved.value.weapon[0].delivery).toEqual({ kind: 'guided', volley_max: 2 });
    expect([...result.resolved.provenance.fields.keys()].some(key => key.includes('beam-a') && key.endsWith('damage_per_sec'))).toBe(false);
  });
  it('inspects and updates legacy family syntax with existing AI intact', () => {
    const legacy = '# before\n[[weapons_console.phaser_banks]]\nid="fore"\nfacing_deg=0\nfire_arc_deg=270\nbeam_range=40\nbeam_damage_per_sec=32\nbeam_duration_secs=6\ncooldown_secs=6\n[weapons_console.phaser_banks.impact]\ndamage_type="laser"\npenetration=20\n[weapons_console.phaser_banks.ai]\nidle=true\n[appearance]\ncolor=[1,2,3]\n';
    const current = draft(legacy), weapons = inspectWeapons(current, dependencies, path).weapons;
    expect(weapons[0].weapon.delivery).toMatchObject({ kind: 'beam', damage_per_sec: 32 });
    weapons[0].weapon.mount.range = 45;
    const after = operation(current, { section: weapons[0].section, originalId: 'fore', weapon: weapons[0].weapon });
    expect(parse(after).weapons_console.phaser_banks[0]).toMatchObject({ beam_range: 45, ai: { idle: true } });
    expect(after.slice(after.indexOf('[appearance]'))).toBe(legacy.slice(legacy.indexOf('[appearance]')));
  });
  it('keeps guided range tied to shared projectile tuning', () => {
    const current = draft('[torpedoes]\nspeed=30\nlifespan=10\n[[torpedoes.tubes]]\nid="tube"\nfacing_deg=0\nfire_arc_deg=90\n[torpedoes.tubes.impact]\ndamage_type="physical"\npenetration=40\n');
    const value = inspectWeapons(current, dependencies, path).weapons[0].weapon; expect(value.mount.range).toBe(300);
    value.mount.range = 40;
    expect(() => operation(current, { section: 'torpedoes.tubes', originalId: 'tube', weapon: value })).toThrow(/Guided range/);
  });
  it('uses supported guided range defaults and explicit idle AI for new equipment', () => {
    const current = draft('[torpedoes]\ncount=4\n[[torpedoes.tubes]]\nid="tube"\nfacing_deg=0\nfire_arc_deg=90\n[torpedoes.tubes.impact]\ndamage_type="physical"\npenetration=40\n');
    expect(inspectWeapons(current, dependencies, path).weapons[0].weapon.mount.range).toBe(300);
    for (const kind of ['beam', 'bolt', 'guided']) expect(defaultWeapon(kind).ai).toEqual({ idle: true });
  });
  it('refuses inline selections, duplicate identities and nonfinite mounts', () => {
    const current = draft('weapon=[{id="beam-a",mount={facing_deg=0,fire_arc_deg=90,range=40},impact={damage_type="laser",penetration=20},delivery={kind="beam",damage_per_sec=10}}]\n');
    expect(() => operation(current, { section: 'weapon', originalId: 'beam-a', weapon: edited(current) })).toThrow(/inline/);
    const value = defaultWeapon(); value.id = 'beam-a'; expect(() => operation(draft(), { weapon: value })).toThrow(/already exists/);
    value.id = 'new'; value.mount.range = NaN; expect(() => operation(draft(), { weapon: value })).toThrow(/finite/);
  });
  it('round trips history and archive; authoritative refusal never lands', async () => {
    const current = draft(), runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    await applyWeaponOperation({ draft: current, runtime, operation: { path, section: 'weapon', originalId: 'beam-a', weapon: edited(current) } });
    const after = current.read(path); current.undo(); expect(current.read(path)).toBe(source); current.redo(); expect(current.read(path)).toBe(after);
    expect(inspectWeapons(new WorkshopDocument(current.archive()), dependencies, path).weapons[0].weapon.mount.range).toBe(50);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ severity: 'error', message: 'Unsupported pattern' }] });
    await expect(applyWeaponOperation({ draft: current, runtime, operation: { path, section: 'weapon', originalId: 'beam-a', weapon: edited(current) } })).rejects.toThrow();
    expect(current.read(path)).toBe(after); expect(current.snapshot().history.undo).toHaveLength(1);
  });
  it('refuses edits when the draft changes during validation', async () => {
    const current = draft(); let resolve; const runtime = { dependencies: async () => dependencies, validate: () => new Promise(done => { resolve = done; }) };
    const pending = applyWeaponOperation({ draft: current, runtime, dependencies, operation: { path, section: 'weapon', originalId: 'beam-a', weapon: edited(current) } });
    await Promise.resolve(); await Promise.resolve();
    current.apply([{ path, before: source, after: source + '# intervening edit\n' }]); resolve({ accepted: true, findings: [] });
    await expect(pending).rejects.toThrow(/stale/); expect(current.read(path)).toBe(source + '# intervening edit\n');
  });
});
