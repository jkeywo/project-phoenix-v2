import { describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { fittingFixture } from './fitting-fixture.js';
import { damageFixture } from './damage-fixture.js';
import { WorkshopDocument } from '../workshop-document.js';
import { createStoreZip } from '../mod-pack-export.js';
import { resolveTemplate } from '../entity-includes.js';
import { applyFittingOperation, healthyEquipment, prepareFittingOperation, validateFitting } from '../workshop-fitting.js';
import { prepareResourceOperation } from '../workshop-resources.js';
import { prepareShipOperation } from '../workshop-ship-authoring.js';
const path = 'assets/entities/test.toml', base = 'assets/entities/base.toml', deps = { base_files: {}, packs: [] };
const make = text => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text }]));
describe('closed physical fitting authoring', () => {
  it('keeps optional payloads in exact defaults and validates sidegrade duration', () => {
    const { value, profile } = fittingFixture();
    profile.equipment[1].hardware.impact.emp = { duration_secs: 6 };
    expect(() => validateFitting(profile, value)).not.toThrow();
    profile.equipment[0].hardware.impact.emp = { duration_secs: 6 };
    expect(() => validateFitting(profile, value)).toThrow(/reproduce/);
    delete profile.equipment[0].hardware.impact.emp;
    profile.equipment[1].hardware.impact.emp.duration_secs = 0;
    expect(() => validateFitting(profile, value)).toThrow(/EMP/);
  });
  it('accepts both actual Cruiser beam and canonical reactor defaults', () => {
    const { value } = damageFixture();
    expect(value.fitting.slot.map(slot => slot.family)).toEqual(['beam', 'reactor']);
    expect(() => validateFitting(value.fitting, value)).not.toThrow();
  });
  it('roundtrips actual hardware, stored conditions and exact-source history through authoritative acceptance', async () => {
    const { value, profile } = fittingFixture(); expect(() => validateFitting(profile, value)).not.toThrow();
    const source = '# header\n' + stringify(value) + '\n# unaffected tail\n', draft = make(source), next = structuredClone(profile);
    next.equipment[1].hardware.continuous_cost_per_sec = 0.75;
    const runtime = { validate: vi.fn(async () => ({ accepted: true })) };
    await applyFittingOperation({ draft, runtime, dependencies: deps, operation: { path, value: next } });
    const after = draft.read(path); expect(after.startsWith('# header\n')).toBe(true); expect(after).toContain('# unaffected tail\n');
    expect(parse(after).fitting).toEqual(next); expect(parse(after).energy).toEqual(value.energy);
    draft.undo(); expect(draft.read(path)).toBe(source); draft.redo(); expect(draft.read(path)).toBe(after);
    expect(parse(new WorkshopDocument(draft.archive()).read(path)).fitting).toEqual(next);
    runtime.validate.mockResolvedValueOnce({ accepted: false, findings: [{ message: 'unsafe actual refit' }] });
    next.equipment[1].work.work_secs = 12;
    await expect(applyFittingOperation({ draft, runtime, dependencies: deps, operation: { path, value: next } })).rejects.toThrow();
    expect(draft.read(path)).toBe(after); expect(draft.snapshot().history.undo).toHaveLength(1);
  });
  it('materialises the whole inherited profile and clears stale inherited arrays/provenance', () => {
    const { value, profile } = fittingFixture(), baseSource = stringify(value), source = 'includes=["base.toml"]\n# child\n', draft = make(source);
    const dependencies = { base_files: { [base]: baseSource }, packs: [] }, next = structuredClone(profile); next.equipment = next.equipment.slice(0, 1); next.assembly = next.assembly.slice(0, 1);
    expect(() => prepareFittingOperation(draft, dependencies, { path, value: next })).toThrow(/materialise/);
    const after = prepareFittingOperation(draft, dependencies, { path, value: next, materialise: true }).changes[0].after;
    const result = resolveTemplate(path, { [path]: after, [base]: baseSource }, parse);
    expect(result.resolved.value.fitting).toEqual(next); expect(dependencies.base_files[base]).toBe(baseSource);
    expect([...result.resolved.provenance.fields.keys()].some(key => key.includes('efficient'))).toBe(false);
    expect(() => validateFitting({}, value)).not.toThrow();
  });
  it.each([
    profile => { profile.slot[0].system = 'repair'; },
    profile => { profile.slot[0].operation_component = 'intake'; },
    profile => { profile.equipment[0].hardware.beam_range += 1; },
    profile => { profile.equipment[1].hardware.cycle_jitter = 0.999999999; },
    profile => { profile.equipment[1].hardware.code = 'extension'; },
    profile => { profile.equipment[1].work.manual_work_fraction = 0; },
    profile => { profile.equipment[1].work.requirements[0].service = 'absent'; },
    profile => { profile.assembly[1].location = { kind: 'in_work', job: 1, compartment: 'weapons' }; },
    profile => { profile.assembly[0].condition = healthyEquipment(); },
    profile => { profile.assembly[1].condition.condition = { kind: 'patched', factor: 0 }; },
    profile => { profile.assembly[1].condition.dose = 1; },
    profile => { profile.assembly[1].condition.condition.kind = 'damaged'; },
    profile => { profile.assembly[1].condition = { condition: { kind: 'patched', factor: 0.73 }, dose: 0, last_source: 'actual-hit' }; },
    profile => { profile.assembly.push({ ...structuredClone(profile.assembly[1]), id: 'second-spare' }); },
    profile => { profile.assembly = profile.assembly.slice(1); },
  ])('refuses unsupported/impossible fitting %# before accepting source', mutate => {
    const { value, profile } = fittingFixture(); mutate(profile); expect(() => validateFitting(profile, value)).toThrow();
  });
  it('guards parts endpoints on a fitting-only hull', () => {
    const { value } = fittingFixture(); delete value.production; delete value.recovery; delete value.recoverable; delete value.port_service;
    const source = stringify(value), draft = make(source);
    expect(() => prepareResourceOperation(draft, deps, { path, type: 'service-set', name: 'machinery_parts', endpoint: { kind: 'capacity', id: 'absent' } })).toThrow(/Fitting/);
    expect(draft.read(path)).toBe(source);
  });
  it('protects actual slot references during ordinary System editing', () => {
    const { source, value, schema } = fittingFixture(), draft = make(source);
    expect(() => prepareShipOperation(draft, deps, schema, { path, type: 'system-remove', id: value.fitting.slot[0].system })).toThrow(/Fitting/);
    expect(draft.read(path)).toBe(source);
  });
});
