import { describe, it, expect, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { defaultShieldArc, inspectShields, prepareShieldOperation, applyShieldOperation } from '../workshop-shields.js';
import { resolveTemplate } from '../entity-includes.js';
import { shieldSource } from './shield-fixture.js';
import { authoredAndGeneratedSystems } from '../workshop-generated-systems.js';
const path = 'assets/entities/ship.toml', base = 'assets/entities/base.toml';
const dependencies = { base_files: {}, packs: [] };
function draft(source = shieldSource()) { return new WorkshopDocument(createStoreZip([
  { path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text: source },
])); }
const prepare = (current, operation, deps = dependencies) => prepareShieldOperation(current, deps, { path, ...operation }).changes[0].after;
const arcOf = current => inspectShields(current, dependencies, path).arcs[0];

describe('Workshop shield work and composition', () => {
  it('edits costs while preserving custom AI and unrelated authored source', () => {
    const current = draft(), before = current.read(path), arc = arcOf(current);
    arc.ai = { custom: 'retained for authoritative validation' }; arc.energy.upkeep_per_sec = 0.2;
    const after = prepare(current, { originalId: arc.id, arc });
    expect(parse(after).shield_arc[0]).toMatchObject({ energy: { upkeep_per_sec: 0.2 }, ai: arc.ai });
    expect(after.slice(0, after.indexOf('[[shield_arc]]'))).toBe(before.slice(0, before.indexOf('[[shield_arc]]')));
    expect(after).toContain('# unrelated footer');
  });
  it('creates arc, canonical reserve and allocation group in one proposed transaction', () => {
    const current = draft(), arc = { ...defaultShieldArc(), id: 'port', label: 'Port' };
    arc.energy.recovery_targets = ['shield-arc-aft'];
    const after = parse(prepare(current, { arc }));
    expect(after.shield_arc.at(-1)).toMatchObject({ id: 'port', energy: arc.energy });
    expect(after.energy.consumer.at(-1)).toMatchObject({ system: 'shield-arc-port', initial_energy: 50, continuous_cost_per_sec: 0, discrete_cost: 0, reserve: { capacity: 100 } });
    expect(after.energy.group.at(-1).id).toBe('energy-shield-arc-port');
    expect(after.power_groups['energy-shield-arc-port']).toMatchObject({ min_level: 0, max_level: 4 });
    expect(after.shield_arc.at(-1).ai).not.toEqual(defaultShieldArc().ai);
  });
  it('normalizes underscore ids like the runtime and rejects normalized collisions', () => {
    const after = prepare(draft(), { arc: { ...defaultShieldArc(), id: 'port_aux', label: 'Auxiliary' } });
    const value = parse(after);
    expect(value.energy.consumer.at(-1).system).toBe('shield-arc-port-aux');
    expect(authoredAndGeneratedSystems(value).find(system => system.id === 'shield-arc-port-aux'))
      .toMatchObject({ kind: 'shield_arc', console: 'generated-shields', power_group: 'energy-shield-arc-port-aux' });
    expect(inspectShields(draft(after), dependencies, path).arcs.at(-1).id).toBe('port_aux');
    expect(() => prepare(draft(after), { arc: { ...defaultShieldArc(), id: 'port-aux' } })).toThrow(/already exists/);
  });
  it('removes the canonical reserve and recovery links together, preserving allocation controls', () => {
    const value = parse(shieldSource()); value.shield_arc[1].energy.recovery_targets = ['shield-arc-fore'];
    value.system = [{ id: 'reactor', kind: 'power_reactor', config: { ai: { channel: 'flow-energy-shield-arc-fore' } } }];
    const after = parse(prepare(draft(stringify(value)), { originalId: 'fore', remove: true, resetAi: true }));
    expect(after.shield_arc.map(arc => arc.id)).toEqual(['aft']);
    expect(after.shield_arc[0].energy.recovery_targets).toEqual([]);
    expect(after.energy.consumer.map(row => row.system)).toEqual(['shield-arc-aft']);
    expect(after.power_groups['energy-shield-arc-fore']).toBeDefined();
  });
  it('refuses missing hardware, nonfinite costs and heterogeneous recovery exploits', () => {
    expect(() => prepare(draft('# empty\n'), { arc: { ...defaultShieldArc(), id: 'fore' } })).toThrow(/reactor/);
    const current = draft(), arc = arcOf(current); arc.energy.upkeep_per_sec = NaN;
    expect(() => prepare(current, { originalId: arc.id, arc })).toThrow(/finite/);
    arc.energy.upkeep_per_sec = 0; arc.energy.recovery_targets = ['shield-arc-aft']; arc.energy.recovery_energy_per_hp = 0.5;
    expect(() => prepare(current, { originalId: arc.id, arc })).toThrow(/less energy/);
    arc.energy.recovery_energy_per_hp = 0.1; arc.energy.recovery_targets = ['foreign-ship::shield-arc-aft'];
    expect(() => prepare(current, { originalId: arc.id, arc })).toThrow(/canonical local reserve/);
  });
  it('requires explicit inherited materialisation and uses keyed removal tombstones', () => {
    const source = shieldSource(), deps = { base_files: { [base]: source }, packs: [] }, current = draft('includes=["base.toml"]\n# child\n');
    const arc = inspectShields(current, deps, path).arcs[0]; arc.energy.upkeep_per_sec = 0.2;
    expect(() => prepare(current, { originalId: arc.id, arc }, deps)).toThrow(/materialise/);
    const after = prepare(current, { originalId: arc.id, remove: true, materialise: true }, deps);
    const result = resolveTemplate(path, { [path]: after, [base]: source }, parse);
    expect(result.ok).toBe(true); expect(result.resolved.value.shield_arc.map(row => row.id)).toEqual(['aft']);
    expect(result.resolved.value.energy.consumer.map(row => row.system)).toEqual(['shield-arc-aft']);
    expect(deps.base_files[base]).toBe(source);
  });
  it('guards explicit information/authority references before deletion', () => {
    const current = draft(shieldSource() + '\n[[console]]\nid="science"\n[[console.feed]]\nsource="shield-arc-fore"\nkind="ShieldArc"\n');
    expect(() => prepare(current, { originalId: 'fore', remove: true })).toThrow(/references from console/);
  });
  it('keeps history and exported source coherent and never lands authoritative refusal', async () => {
    const current = draft(), before = current.read(path), runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    const arc = arcOf(current); arc.energy.upkeep_per_sec = 0.2; arc.focus_available = false;
    await applyShieldOperation({ draft: current, runtime, operation: { path, originalId: arc.id, arc } });
    const after = current.read(path); current.undo(); expect(current.read(path)).toBe(before); current.redo(); expect(current.read(path)).toBe(after);
    expect(parse(new WorkshopDocument(current.archive()).read(path)).shield_arc[0].energy.upkeep_per_sec).toBe(0.2);
    expect(parse(new WorkshopDocument(current.archive()).read(path)).shield_arc[0].focus_available).toBe(false);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'Unsupported shield AI reference' }] });
    await expect(applyShieldOperation({ draft: current, runtime, operation: { path, originalId: arc.id, arc } })).rejects.toThrow();
    expect(current.read(path)).toBe(after); expect(current.snapshot().history.undo).toHaveLength(1);
  });
});
