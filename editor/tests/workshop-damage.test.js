import { describe, it, expect, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { inspectDamage, prepareDamageOperation, applyDamageOperation } from '../workshop-damage.js';
import { defaultDamageComponent, defaultEmpResistance, validateEmpResistance, validateEmpPayload } from '../workshop-damage-model.js';
import { damageFixture } from './damage-fixture.js';
import { resolveTemplate } from '../entity-includes.js';
const path = 'assets/entities/ship.toml', base = 'assets/entities/base.toml', dependencies = { base_files: {}, packs: [] };
function draft(source = damageFixture().source) { return new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source }])); }
const prepare = (current, operation, deps = dependencies) => prepareDamageOperation(current, deps, { path, ...operation }).changes[0].after;

it('edits a closed EMP receiver profile atomically without changing materials or remedies', () => {
  const current = draft(), original = inspectDamage(current, dependencies, path).damage;
  const damage = { ...original, emp: defaultEmpResistance() };
  const after = prepare(current, { type: 'damage-profile', damage });
  expect(parse(after).damage).toEqual(damage); expect(parse(after).system).toEqual(parse(current.read(path)).system);
  for (const invalid of [{ ...damage.emp, max_stacks: 0 }, { ...damage.emp, resistance_per_stack: '1' }, { ...damage.emp, decay_secs: Infinity }, { ...damage.emp, bindings: ['operation', 'operation'] }]) expect(() => validateEmpResistance(invalid)).toThrow();
  for (const invalid of [{ duration_secs: 0 }, { duration_secs: Infinity }, { duration_secs: 1, factor: 0.5 }]) expect(() => validateEmpPayload(invalid)).toThrow();
});

describe('damage authoring source contracts', () => {
  it('edits material/recipe data without touching machinery, comments or layout', () => {
    const current = draft(), before = current.read(path), read = inspectDamage(current, dependencies, path);
    read.damage.materials.armour.resistance.physical = 3;
    const after = prepare(current, { type: 'damage-profile', damage: read.damage });
    expect(parse(after).damage.materials.armour.resistance.physical).toBe(3);
    expect(parse(after).system).toEqual(parse(before).system); expect(parse(after).interior).toEqual(parse(before).interior);
  });
  it('edits actual System components while retaining its ordinary AI and peer definitions', () => {
    const current = draft(), before = parse(current.read(path)), rows = before.system.find(system => system.id === 'power-reactor').damage_component;
    rows[0].degraded_factor = 0.3;
    const after = parse(prepare(current, { type: 'damage-components', system: 'power-reactor', components: rows }));
    expect(after.system.find(system => system.id === 'power-reactor').damage_component[0].degraded_factor).toBe(0.3);
    expect(after.system.find(system => system.id === 'power-reactor').config).toEqual(before.system.find(system => system.id === 'power-reactor').config);
    expect(after.system.filter(system => system.id !== 'power-reactor')).toEqual(before.system.filter(system => system.id !== 'power-reactor'));
  });
  it('writes generated shield equipment once through its actual arc definition', () => {
    const current = draft(), before = parse(current.read(path)), rows = structuredClone(before.shield_arc[0].damage_component);
    rows[0].field_replaceable = false;
    const after = parse(prepare(current, { type: 'damage-components', system: 'shield-arc-fore', components: rows }));
    expect(after.shield_arc[0].damage_component[0].field_replaceable).toBe(false);
    expect(after.system.some(system => system.id === 'shield-arc-fore')).toBe(false);
    expect(after.shield_arc[0].ai).toEqual(before.shield_arc[0].ai);
  });
  it('refuses unsupported bindings, imaginary part services, missing remedies and invalid structure', () => {
    const current = draft(), read = inspectDamage(current, dependencies, path);
    const rows = [defaultDamageComponent('unsupported')];
    expect(() => prepare(current, { type: 'damage-components', system: 'repair', components: rows })).toThrow(/component/);
    read.damage.remedies['machinery-full'].requirements[0].service = 'imaginary';
    expect(() => prepare(current, { type: 'damage-profile', damage: read.damage })).toThrow(/actual local capacity/);
    const damage = inspectDamage(current, dependencies, path).damage; delete damage.remedies.structural;
    expect(() => prepare(current, { type: 'damage-profile', damage })).toThrow(/structural remedy/);
  });
  it('requires explicit inherited materialisation and leaves dependency bytes intact', () => {
    const value = damageFixture().value; value.damage.materials.obsolete = structuredClone(value.damage.materials.light);
    const source = stringify(value), current = draft('includes=["base.toml"]\n# local\n'), deps = { base_files: { [base]: source }, packs: [] };
    const damage = inspectDamage(current, deps, path).damage; damage.materials.light.residual_fraction = 0.2; delete damage.materials.obsolete;
    expect(() => prepare(current, { type: 'damage-profile', damage }, deps)).toThrow(/materialise/);
    const after = prepare(current, { type: 'damage-profile', damage, materialise: true }, deps);
    expect(after.startsWith('includes=["base.toml"]\n# local\n')).toBe(true); expect(deps.base_files[base]).toBe(source);
    const resolved = resolveTemplate(path, { [path]: after, [base]: source }, parse).resolved;
    expect(resolved.value.damage.materials.obsolete).toBeUndefined();
    expect([...resolved.provenance.fields.keys()].some(field => field.includes('damage.materials.obsolete'))).toBe(false);
  });
  it('permits distinct fault components sharing one operation binding', () => {
    const current = draft(), rows = inspectDamage(current, dependencies, path).systems.find(row => row.id === 'repair').damage_component;
    rows.push({ ...defaultDamageComponent(), id: 'secondary-tools' });
    const after = parse(prepare(current, { type: 'damage-components', system: 'repair', components: rows }));
    expect(after.system.find(row => row.id === 'repair').damage_component.filter(row => row.binding === 'operation')).toHaveLength(2);
  });
  it('converts explicit legacy structural data and impacts as one finite authored profile', () => {
    const value = damageFixture().value; delete value.damage;
    value.hull = { system_hull: [{ system_id: 'old-core', max_hp: 590 }] };
    for (const system of value.system) delete system.damage_component;
    for (const arc of value.shield_arc) { delete arc.damage_component; arc.hull_max_hp = 50; }
    for (const weapon of value.weapon) delete weapon.impact;
    value.interior.repair_placements = { 'old-core': 'engineering' };
    const after = parse(prepare(draft(stringify(value)), { type: 'damage-create' }));
    expect(after.hull).toBeUndefined(); expect(after.interior.repair_placements).toBeUndefined();
    expect(after.interior.compartment.reduce((sum, room) => sum + room.max_hp, 0)).toBeCloseTo(590);
    expect(after.shield_arc.every(arc => arc.hull_max_hp === undefined)).toBe(true);
    expect(after.weapon.every(weapon => weapon.impact && Number.isFinite(weapon.impact.penetration))).toBe(true);
  });
  it('preserves source/history on authoritative rejection and round trips accepted edits', async () => {
    const current = draft(), before = current.read(path), runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: false, findings: [{ message: 'unsupported material' }] })) };
    const damage = inspectDamage(current, dependencies, path).damage; damage.materials.armour.residual_fraction = 0.2;
    await expect(applyDamageOperation({ draft: current, runtime, operation: { path, type: 'damage-profile', damage } })).rejects.toThrow();
    expect(current.read(path)).toBe(before); expect(current.snapshot().history.undo).toHaveLength(0);
    runtime.validate.mockResolvedValue({ accepted: true, findings: [] });
    await applyDamageOperation({ draft: current, runtime, operation: { path, type: 'damage-profile', damage } });
    const after = current.read(path); current.undo(); expect(current.read(path)).toBe(before); current.redo(); expect(current.read(path)).toBe(after);
    expect(inspectDamage(new WorkshopDocument(current.archive()), dependencies, path).damage).toEqual(damage);
  });
});
