import { describe, it, expect, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { createEnergyModel, inspectEnergy, prepareEnergyOperation, applyEnergyOperation } from '../workshop-energy.js';
import { resolveTemplate } from '../entity-includes.js';
import { stripRemovals } from '../entity-includes.js';
import { applyShipOperation } from '../workshop-ship-authoring.js';
const path = 'assets/entities/energy-ship.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const hardware = { system: [{ id: 'impulse', kind: 'impulse', power_group: 'propulsion' }, { id: 'power-reactor', kind: 'power_reactor', power_group: 'reactor' }, { id: 'power-battery', kind: 'power_battery' }], power_groups: { propulsion: {}, reactor: {} } };
const source = '# keep top\n' + stringify({ energy: createEnergyModel(hardware) }) + '# keep appearance\n[appearance]\ncolor=[1,2,3]\n';
function draft(text = source) { return new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n' }, { path, text }])); }
describe('Workshop energy authoring', () => {
  it('explicit starter model covers powered consumers without a second reactor/battery', () => {
    const model = createEnergyModel(hardware); expect(model.consumer.map(item => item.system)).toEqual(['impulse']);
    expect(model.group.map(item => item.id)).toEqual(['propulsion', 'reactor']);
    expect(model.reactor.coolant).toEqual({ service: 'coolant' }); expect(model.consumer[0].coolant).toEqual({ service: 'coolant' });
  });
  it('creates the first energy hardware and ordinary Power tree in one accepted history edit', async () => {
    const value = { ...hardware, resource_services: { coolant: { kind: 'capacity', id: 'tank' } },
      infrastructure: { capacity: [{ id: 'tank', amount: 10 }] } };
    value.system[1].config = { ai: { idle: true } };
    const before = stringify(value), current = draft(before), runtime = { dependencies: async () => deps, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    await applyEnergyOperation({ draft: current, runtime, operation: { path, energy: createEnergyModel(value) } });
    const after = current.read(path), parsed = stripRemovals(parse(after));
    expect(parsed.energy.consumer.map(row => row.system)).toEqual(['impulse']);
    expect(parsed.system[1].config.ai).toMatchObject({ root: 'power' });
    expect(parsed.system[1].config.ai.tree[0].state[0].task[0].action.kind).toBe('write_number');
    expect(current.snapshot().history.undo).toHaveLength(1); current.undo(); expect(current.read(path)).toBe(before);
  });
  it('adds and removes machinery together with its local reserve in single history edits', async () => {
    const value = { ...hardware, tags: ['ship'], station: [{ id: 'engineering' }],
      console: [{ id: 'engineering', home_station: 'engineering', screen: 'engineering.html' }],
      system: [{ id: 'existing', kind: 'helm_thrust', console: 'engineering', power_group: 'propulsion' }],
      resource_services: { coolant: { kind: 'capacity', id: 'tank' } }, infrastructure: { capacity: [{ id: 'tank', amount: 10 }] } };
    value.energy = createEnergyModel(value);
    const before = stringify(value), current = draft(before), runtime = { dependencies: async () => deps,
      shipSchema: async () => ({ system_kinds: ['helm_thrust', 'helm_radar'] }), validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    await applyShipOperation({ draft: current, runtime, operation: { path, type: 'system-add', id: 'new-radar', kind: 'helm_radar', console: 'engineering', power_group: 'propulsion' } });
    const added = current.read(path); expect(parse(added).energy.consumer.map(row => row.system)).toEqual(['existing', 'new-radar']);
    expect(parse(added).energy.reactor).toEqual(value.energy.reactor); expect(current.snapshot().history.undo).toHaveLength(1);
    await applyShipOperation({ draft: current, runtime, operation: { path, type: 'system-remove', id: 'new-radar' } });
    expect(parse(current.read(path)).energy.consumer.map(row => row.system)).toEqual(['existing']);
    current.undo(); expect(current.read(path)).toBe(added); current.undo(); expect(current.read(path)).toBe(before);
  });
  it('refuses mandatory machinery without groups, including generated physical shield arcs', () => {
    expect(() => createEnergyModel({ system: [{ id: 'drive', kind: 'helm_thrust' }] })).toThrow(/Assign authored power groups.*drive/);
    expect(() => createEnergyModel({ system: [{ id: 'shields', kind: 'shields' }], shield_arc: [{ id: 'fore' }] })).toThrow(/shield-arc-fore/);
    const model = createEnergyModel({ system: [{ id: 'shields', kind: 'shields', power_group: 'protection' },
      { id: 'shield-arc-fore', kind: 'shield_arc', power_group: 'protection' }], shield_arc: [{ id: 'fore' }], power_groups: { protection: {} } });
    expect(model.consumer.map(row => row.system)).toEqual(['shield-arc-fore']);
  });
  it.each(['\n', '\r\n'])('replaces a full energy profile preserving unrelated exact source %j', nl => {
    const before = source.replace(/\n/g, nl), current = draft(before), energy = inspectEnergy(current, deps, path).energy;
    energy.consumer[0].reserve.capacity = 200;
    const after = prepareEnergyOperation(current, deps, { path, energy }).changes[0].after;
    expect(after.startsWith('# keep top' + nl)).toBe(true);
    expect(after.slice(after.indexOf('[appearance]'))).toBe(before.slice(before.indexOf('[appearance]')));
    expect(parse(after).energy.consumer[0].reserve.capacity).toBe(200);
  });
  it('requires materialisation for inherited energy and leaves its dependencies untouched', () => {
    const current = draft('includes=["base.toml"]\n# child\n'), dependencies = { base_files: { [base]: source }, packs: [] };
    const energy = inspectEnergy(current, dependencies, path).energy; energy.reactor.initial_energy = 70;
    expect(() => prepareEnergyOperation(current, dependencies, { path, energy })).toThrow(/materialise/);
    const after = prepareEnergyOperation(current, dependencies, { path, energy, materialise: true }).changes[0].after;
    expect(after.startsWith('includes=["base.toml"]\n# child\n')).toBe(true);
    expect(resolveTemplate(path, { [path]: after, [base]: source }, parse).resolved.value.energy.reactor.initial_energy).toBe(70);
    expect(dependencies.base_files[base]).toBe(source);
  });
  it('atomic materialisation clears inherited parent links, old coolant variants and provenance', () => {
    const inherited = createEnergyModel(hardware); inherited.group[0].parent = 'reactor';
    inherited.consumer[0].coolant = { provider: { kind: 'capacity', id: 'old-tank' } };
    const parent = stringify({ energy: inherited }), current = draft('includes=["base.toml"]\n'), dependencies = { base_files: { [base]: parent }, packs: [] };
    const energy = inspectEnergy(current, dependencies, path).energy; delete energy.group[0].parent; energy.consumer[0].coolant = { service: 'coolant' };
    const after = prepareEnergyOperation(current, dependencies, { path, energy, materialise: true }).changes[0].after;
    const resolved = resolveTemplate(path, { [path]: after, [base]: parent }, parse).resolved;
    expect(resolved.value.energy).toEqual(energy);
    expect([...resolved.provenance.fields.keys()].some(key => key.includes('coolant.provider') || key.endsWith('.parent'))).toBe(false);
  });
  it('removes every selected energy span without changing intervening unrelated tables', () => {
    const text = '[energy.reactor]\ninitial_energy=1\n[appearance]\ncolor=[1,2,3] # retain\n[[energy.consumer]]\nsystem="x"\n[transform]\nposition=[1,2,3]\n';
    const after = prepareEnergyOperation(draft(text), deps, { path, energy: createEnergyModel(hardware) }).changes[0].after;
    expect(after).toContain('[appearance]\ncolor=[1,2,3] # retain\n'); expect(after).toContain('[transform]\nposition=[1,2,3]\n');
    expect(parse(after).energy.consumer.map(item => item.system)).toEqual(['impulse']);
  });
  it('refuses inline energy instead of writing a duplicate table', () => {
    expect(() => prepareEnergyOperation(draft('energy={reactor={initial_energy=1}}\n'), deps, { path, energy: createEnergyModel(hardware) })).toThrow(/inline/);
  });
  it('round trips accepted history and archive; authoritative refusal leaves source/history intact', async () => {
    const current = draft(), energy = inspectEnergy(current, deps, path).energy; energy.reactor.initial_energy = 70;
    const runtime = { dependencies: async () => deps, validate: vi.fn(async () => ({ accepted: true, findings: [] })) };
    await applyEnergyOperation({ draft: current, runtime, operation: { path, energy } });
    const after = current.read(path); current.undo(); expect(current.read(path)).toBe(source); current.redo(); expect(current.read(path)).toBe(after);
    expect(inspectEnergy(new WorkshopDocument(current.archive()), deps, path).energy.reactor.initial_energy).toBe(70);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ severity: 'error', message: 'coolant missing' }] });
    energy.reactor.coolant = { service: 'absent' };
    await expect(applyEnergyOperation({ draft: current, runtime, operation: { path, energy } })).rejects.toThrow();
    expect(current.read(path)).toBe(after); expect(current.snapshot().history.undo).toHaveLength(1);
  });
  it('rejects stale validation after the source changes', async () => {
    const current = draft(); let resolve; const runtime = { dependencies: async () => deps, validate: () => new Promise(done => { resolve = done; }) };
    const pending = applyEnergyOperation({ draft: current, runtime, dependencies: deps, operation: { path, energy: createEnergyModel(hardware) } });
    await Promise.resolve(); await Promise.resolve(); current.apply([{ path, before: source, after: source + '# newer\n' }]);
    resolve({ accepted: true, findings: [] }); await expect(pending).rejects.toThrow(/stale/); expect(current.read(path)).toBe(source + '# newer\n');
  });
});
